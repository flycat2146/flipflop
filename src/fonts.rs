//! Put back the CJK font fallbacks that fontique's Android backend drops.
//!
//! Slint lays text out through parley/fontique. On Android, fontique builds its
//! script/locale fallback map from `/system/etc/fonts.xml` — but only for families
//! that carry a `postScriptName` attribute, while Android declares its Chinese,
//! Japanese and Korean families by *file name*:
//!
//! ```xml
//! <family lang="zh-Hans">
//!     <font weight="400" style="normal" index="2">NotoSansCJK-Regular.ttc</font>
//! </family>
//! ```
//!
//! Those entries are silently skipped (upstream left TODOs about exactly this in
//! `fontique/src/backend/android.rs`), so nothing maps the Han script to a font that
//! covers it. The default family (Roboto/Noto Sans) has no Han, Kana or Hangul
//! coverage, and with no fallback to try after it the shaper has nowhere to go:
//! every Chinese character on an Android screen renders as an empty box.
//!
//! That is a *rendering* problem only. The strings themselves stay correct UTF-8, so
//! a message typed on the phone arrives intact and displays fine on the desktop peer
//! it was sent to.
//!
//! Slint exposes the very collection it renders with, so the missing mappings can be
//! restored from the app: point the Han script at the CJK families the system already
//! reports. `fontique`'s `Query::set_fallbacks` adds the Han fallback to *every*
//! query, so registering it there is enough to fix all CJK text.

use std::collections::HashSet;
use std::path::PathBuf;

use slint::fontique_011::fontique::{Blob, Collection, FallbackKey, FamilyId, Script};

/// Marks a family name (`Noto Sans CJK SC`, `MiSans`, …) or a font file name
/// (`NotoSansCJK-Regular.ttc`, `DroidSansFallbackFull.ttf`, …) as CJK-capable.
const CJK_HINTS: &[&str] = &[
    "cjk",
    "sans sc",
    "sans tc",
    "sanssc",
    "sanstc",
    "serif sc",
    "serif tc",
    "source han",
    "sourcehan",
    "noto sans sc",
    "noto sans tc",
    "noto serif sc",
    "noto serif tc",
    // Android's own last-resort font for scripts the primary fonts miss.
    "fallback",
    // Vendor fonts that replace the Noto CJK families on Chinese ROMs.
    "misans",
    "harmonyos",
    "oppo sans",
    "oplus",
    "oneplus",
    "samsung one",
    "samsungone",
    "vivo sans",
];

/// The scripts a CJK font is worth mapping to. `Hani` is the one that does the work:
/// fontique appends the Han fallback to every single query, so a family registered
/// there covers Kana and Hangul the same way.
const CJK_SCRIPTS: &[&str] = &["Hani", "Hira", "Kana", "Hang"];

/// Register the device's CJK fonts as fallbacks. Call once, before the first window
/// is created, so no already-shaped (and therefore box-rendered) text is cached.
///
/// Does nothing when the device has no recognisable CJK font at all — the previous
/// behaviour, boxes included, is then the best we can do.
pub fn install_cjk_fallback() {
    let mut collection = slint::fontique_011::shared_collection();

    // Normally the families are already registered — fontique scans `/system/fonts`
    // and only the script -> family mapping is missing. Fall back to reading the
    // files ourselves if no family name looked like a CJK one (a ROM that renamed
    // everything).
    let mut families = families_by_name(&mut collection);
    if families.is_empty() {
        families = register_system_files(&mut collection);
    }
    if families.is_empty() {
        tracing::warn!("no CJK font found on this device; Chinese text will render as boxes");
        return;
    }

    prefer_simplified_chinese(&mut collection, &mut families);
    for script in CJK_SCRIPTS {
        collection.append_fallbacks(
            FallbackKey::new(Script::from_str_unchecked(script), None),
            families.iter().copied(),
        );
    }

    let names: Vec<String> = families
        .iter()
        .filter_map(|id| collection.family_name(*id).map(str::to_owned))
        .collect();
    tracing::info!("CJK fallback fonts: {}", names.join(", "));
}

/// Every family the collection (that is: every font the system reported) whose name
/// looks like a CJK font.
fn families_by_name(collection: &mut Collection) -> Vec<FamilyId> {
    let names: Vec<String> = collection.family_names().map(str::to_owned).collect();
    let mut ids = Vec::new();
    for name in names {
        if !is_cjk_name(&name) {
            continue;
        }
        if let Some(id) = collection.family_id(&name) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Look for CJK font files in the system font directories and register them. Only
/// needed when `families_by_name` came up empty.
fn register_system_files(collection: &mut Collection) -> Vec<FamilyId> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for dir in font_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_cjk_name(file_name) || !seen.insert(path.clone()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            for (id, _) in collection.register_fonts(Blob::from(bytes), None) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            tracing::debug!("registered CJK font {}", path.display());
        }
    }
    ids
}

/// `/system/fonts` holds the AOSP fonts; vendors that would rather not touch
/// `/system` put theirs in `/product/fonts` or `/vendor/fonts`.
fn font_dirs() -> Vec<PathBuf> {
    let root = std::env::var("ANDROID_ROOT").unwrap_or_else(|_| "/system".to_owned());
    vec![
        PathBuf::from(&root).join("fonts"),
        PathBuf::from("/product/fonts"),
        PathBuf::from("/vendor/fonts"),
    ]
}

fn is_cjk_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    CJK_HINTS.iter().any(|hint| lower.contains(hint))
}

/// The Han glyphs are shared between the JP/KR/SC/TC faces of a
/// `NotoSansCJK-Regular.ttc`, so the *first* family in the fallback chain decides
/// which variant the reader gets. Chinese readers expect the simplified shapes.
fn prefer_simplified_chinese(collection: &mut Collection, families: &mut Vec<FamilyId>) {
    let mut ranks = Vec::with_capacity(families.len());
    for id in families.iter() {
        let name = collection
            .family_name(*id)
            .unwrap_or_default()
            .to_lowercase();
        let rank = if name.contains("hans")
            || name.contains(" sc")
            || name.contains("misans")
            || name.contains("harmonyos")
            || name.contains("simplified")
        {
            0
        } else if name.contains("hant")
            || name.contains(" tc")
            || name.contains(" hk")
            || name.contains("traditional")
        {
            1
        } else if name.contains("jp") || name.contains("japan") {
            2
        } else if name.contains("kr") || name.contains("kore") {
            3
        } else {
            // A single-face CJK font (MiSans, DroidSansFallback, …): only one shape
            // to offer, so it may as well come first.
            0
        };
        ranks.push((rank, *id));
    }
    ranks.sort_by_key(|(rank, _)| *rank);
    families.clear();
    families.extend(ranks.into_iter().map(|(_, id)| id));
}
