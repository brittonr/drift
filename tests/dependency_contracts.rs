//! Contracts used by Drift across dependency version updates.
use std::num::NonZeroUsize;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use lru::LruCache;
use scraper::{Html, Selector};

#[test]
fn manifest_base64_round_trip_preserves_bytes() {
    let manifest = br#"{"urls":["https://example.invalid/audio"]}"#;
    assert_eq!(
        STANDARD.decode(STANDARD.encode(manifest)).unwrap(),
        manifest
    );
}

#[test]
fn malformed_manifest_encoding_is_rejected() {
    assert!(STANDARD.decode("not%base64").is_err());
    assert!(STANDARD.decode("Y").is_err());
}

#[test]
fn album_art_cache_evicts_the_least_recent_entry() {
    const CACHE_CAPACITY: usize = 2;
    let mut cache = LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap());
    cache.put("first", "first image");
    cache.put("second", "second image");
    assert_eq!(cache.get("first"), Some(&"first image"));
    cache.put("third", "third image");
    assert!(!cache.contains("second"));
    assert!(cache.contains("first"));
    assert!(cache.contains("third"));
    assert_eq!(cache.len(), CACHE_CAPACITY);
    assert!(cache.get("missing").is_none());
}

#[test]
fn html_selection_preserves_link_text_and_href() {
    let document =
        Html::parse_document(r#"<a class="item" href="/track/song">Song &amp; title</a>"#);
    let selector = Selector::parse("a.item").unwrap();
    let link = document.select(&selector).next().unwrap();
    assert_eq!(link.value().attr("href"), Some("/track/song"));
    assert_eq!(link.text().collect::<String>(), "Song & title");
}

#[test]
fn missing_html_results_and_invalid_selectors_are_safe() {
    let document = Html::parse_document("<div>unrelated content</div>");
    let selector = Selector::parse("a.item").unwrap();
    assert!(document.select(&selector).next().is_none());
    assert!(Selector::parse("[").is_err());
}
