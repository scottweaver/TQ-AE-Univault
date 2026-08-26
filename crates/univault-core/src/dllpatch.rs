//! The community socket-gate patch for `Game.dll` — the one
//! sanctioned write to a game binary (see ARCHITECTURE.md). The
//! game's socketing UI rejects relics/charms on Epic and Legendary
//! items with two conditional jumps after classification compares;
//! NOP-ing the jumps lifts the rarity gate while every other rule
//! (type flags, costs) stays engine-enforced. Byte-exact and
//! same-length, so the patch is its own inverse and the file never
//! changes size. Ported from the public guide the user supplied
//! (Steam Community 2202151189); the shell owns backups, warnings,
//! and the game-update caveat.

/// `cmp eax, 4; je +0x14; cmp eax, 3; je +0x0f` — the Epic (4) and
/// Legendary (3) rejections.
pub const VANILLA_SIGNATURE: [u8; 10] =
    [0x83, 0xF8, 0x04, 0x74, 0x14, 0x83, 0xF8, 0x03, 0x74, 0x0F];

/// The same compares with both jumps NOP'd out.
pub const PATCHED_SIGNATURE: [u8; 10] =
    [0x83, 0xF8, 0x04, 0x90, 0x90, 0x83, 0xF8, 0x03, 0x90, 0x90];

/// What a `Game.dll` image looks like with respect to the patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchState {
    /// Only untouched signature sites present.
    Vanilla { sites: usize },
    /// Only patched sites present.
    Patched { sites: usize },
    /// Both forms present — a partial application (older guides'
    /// multi-site binaries, or an interrupted write).
    Mixed { vanilla: usize, patched: usize },
    /// Neither form present: an unknown game version. Never write.
    Unrecognized,
}

#[must_use]
pub fn inspect(dll: &[u8]) -> PatchState {
    let vanilla = count(dll, &VANILLA_SIGNATURE);
    let patched = count(dll, &PATCHED_SIGNATURE);
    match (vanilla, patched) {
        (0, 0) => PatchState::Unrecognized,
        (sites, 0) => PatchState::Vanilla { sites },
        (0, sites) => PatchState::Patched { sites },
        (vanilla, patched) => PatchState::Mixed { vanilla, patched },
    }
}

/// Patches every vanilla site in place; the count actually changed.
pub fn enable(dll: &mut [u8]) -> usize {
    replace_all(dll, &VANILLA_SIGNATURE, &PATCHED_SIGNATURE)
}

/// Restores every patched site in place; the count actually changed.
pub fn disable(dll: &mut [u8]) -> usize {
    replace_all(dll, &PATCHED_SIGNATURE, &VANILLA_SIGNATURE)
}

fn count(dll: &[u8], pattern: &[u8; 10]) -> usize {
    let mut found = 0;
    let mut from = 0;
    while let Some(at) = find(dll, pattern, from) {
        found += 1;
        from = at + pattern.len();
    }
    found
}

fn replace_all(dll: &mut [u8], from_pattern: &[u8; 10], to_pattern: &[u8; 10]) -> usize {
    let mut changed = 0;
    let mut from = 0;
    while let Some(at) = find(dll, from_pattern, from) {
        dll[at..at + to_pattern.len()].copy_from_slice(to_pattern);
        changed += 1;
        from = at + to_pattern.len();
    }
    changed
}

fn find(haystack: &[u8], needle: &[u8; 10], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(sites: &[&[u8; 10]]) -> Vec<u8> {
        let mut out = vec![0xCC_u8; 7];
        for site in sites {
            out.extend_from_slice(*site);
            out.extend_from_slice(&[0x11, 0x22, 0x33]);
        }
        out
    }

    #[test]
    fn inspect_classifies_all_four_states() {
        assert_eq!(
            inspect(&image(&[&VANILLA_SIGNATURE, &VANILLA_SIGNATURE])),
            PatchState::Vanilla { sites: 2 }
        );
        assert_eq!(
            inspect(&image(&[&PATCHED_SIGNATURE])),
            PatchState::Patched { sites: 1 }
        );
        assert_eq!(
            inspect(&image(&[&VANILLA_SIGNATURE, &PATCHED_SIGNATURE])),
            PatchState::Mixed {
                vanilla: 1,
                patched: 1
            }
        );
        assert_eq!(inspect(&image(&[])), PatchState::Unrecognized);
    }

    #[test]
    fn enable_then_disable_is_byte_identical() {
        let original = image(&[&VANILLA_SIGNATURE, &VANILLA_SIGNATURE, &VANILLA_SIGNATURE]);
        let mut working = original.clone();
        assert_eq!(enable(&mut working), 3);
        assert_eq!(working.len(), original.len());
        assert_eq!(inspect(&working), PatchState::Patched { sites: 3 });
        assert_ne!(working, original);
        assert_eq!(disable(&mut working), 3);
        assert_eq!(working, original);
    }

    #[test]
    fn enable_completes_a_mixed_image_and_never_touches_other_bytes() {
        let mut working = image(&[&VANILLA_SIGNATURE, &PATCHED_SIGNATURE]);
        let fully_patched = image(&[&PATCHED_SIGNATURE, &PATCHED_SIGNATURE]);
        assert_eq!(enable(&mut working), 1);
        assert_eq!(working, fully_patched);
        // Nothing to do on an already-patched image.
        assert_eq!(enable(&mut working), 0);
    }
}
