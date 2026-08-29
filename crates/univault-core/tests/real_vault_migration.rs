//! Migration check against a real vault file and game cache — the
//! path the store takes on its first launch, over real data rather
//! than fixtures. Skipped unless `UNIVAULT_REAL_VAULT` and
//! `UNIVAULT_REAL_CACHE` name them, so CI and a fresh clone stay
//! green without a game install.

use std::collections::BTreeMap;

use univault_core::cache::GameCache;
use univault_core::store::{self, Bucket, Family, VaultStore};
use univault_core::vault::Vault;

#[test]
fn real_vault_migrates_classifies_and_round_trips() {
    let (Some(vault_path), Some(cache_path)) = (
        std::env::var_os("UNIVAULT_REAL_VAULT"),
        std::env::var_os("UNIVAULT_REAL_CACHE"),
    ) else {
        eprintln!("skipped: set UNIVAULT_REAL_VAULT and UNIVAULT_REAL_CACHE");
        return;
    };

    let text = std::fs::read_to_string(&vault_path).unwrap();
    let vault = Vault::from_json(&text).unwrap();
    let db = GameCache::from_bytes(&std::fs::read(&cache_path).unwrap()).unwrap();

    let mut vault_items = 0;
    for sack in &vault.sacks {
        vault_items += sack.items.len();
    }

    let mut vault_store = VaultStore::new();
    let imported = vault_store.import_vault(&vault);
    assert_eq!(imported, vault_items, "every vault item must come across");
    assert_eq!(vault_store.len(), vault_items);

    // Round trip through the store's own file format.
    let json = vault_store.to_json().unwrap();
    let reread = VaultStore::from_json(&json).unwrap();
    assert_eq!(reread, vault_store, "store round trip must be lossless");

    // Classify with the real game cache.
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in vault_store.entries() {
        *counts
            .entry(store::bucket_of(Some(&db), &entry.item).label())
            .or_insert(0) += 1;
    }
    let unknown = counts.get("Unknown").copied().unwrap_or(0);
    eprintln!("--- {vault_items} items from {vault_path:?}");
    eprintln!("--- store json: {} bytes", json.len());
    for (bucket, count) in &counts {
        eprintln!("    {bucket:>14}: {count}");
    }
    eprintln!("--- unknown: {unknown}");

    // Every item must land in exactly one family's bucket list.
    for entry in vault_store.entries() {
        let bucket = store::bucket_of(Some(&db), &entry.item);
        let family = bucket.family();
        assert!(
            family.buckets().contains(&bucket),
            "{bucket:?} missing from {family:?}"
        );
    }
    let listed: usize = Family::ALL
        .into_iter()
        .flat_map(|family| family.buckets().iter().copied())
        .filter(|bucket| *bucket != Bucket::Unknown)
        .count();
    assert_eq!(listed, univault_core::query::ItemCategory::ALL.len());

    // Export back out and confirm TQVaultAE can read what we wrote.
    let items: Vec<_> = vault_store
        .entries()
        .map(|entry| entry.item.clone())
        .collect();
    let exported = store::export_to_vault(items, Some(&db));
    let exported_json = exported.to_json().unwrap();
    let reparsed = Vault::from_json(&exported_json).unwrap();
    let exported_items: usize = reparsed.sacks.iter().map(|sack| sack.items.len()).sum();
    assert_eq!(exported_items, vault_items, "export must lose nothing");
    eprintln!(
        "--- exported into {} sacks, {exported_items} items",
        reparsed.sacks.len()
    );

    // No two items in a sack may overlap at their real footprints.
    for (index, sack) in reparsed.sacks.iter().enumerate() {
        let rects: Vec<(i32, i32, i32, i32)> = sack
            .items
            .iter()
            .map(|entry| {
                let (w, h) = db.item_footprint(&entry.item);
                (entry.item.position.x, entry.item.position.y, w, h)
            })
            .collect();
        for (a_index, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(a_index + 1) {
                let overlaps =
                    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
                assert!(!overlaps, "sack {index}: {a:?} overlaps {b:?}");
            }
        }
    }
}
