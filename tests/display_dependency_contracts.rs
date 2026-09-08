use image::{DynamicImage, Rgba};
use ratatui::{buffer::Buffer, layout::Rect, widgets::StatefulWidget};
use ratatui_image::{picker::Picker, StatefulImage};

const IMAGE_WIDTH: u32 = 4;
const IMAGE_HEIGHT: u32 = 8;
const CELL_WIDTH: u16 = 2;
const CELL_HEIGHT: u16 = 2;
const COLOR_MAX: u8 = u8::MAX;

fn image() -> DynamicImage {
    DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
        Rgba([COLOR_MAX, 0, 0, COLOR_MAX]),
    ))
}

#[test]
fn halfblock_fallback_renders_without_terminal_queries() {
    let mut protocol = Picker::halfblocks().new_resize_protocol(image());
    let area = Rect::new(0, 0, CELL_WIDTH, CELL_HEIGHT);
    let mut buffer = Buffer::empty(area);
    StatefulImage::default().render(area, &mut buffer, &mut protocol);
    let red = ratatui::style::Color::Rgb(COLOR_MAX, 0, 0);
    assert!(buffer
        .content
        .iter()
        .any(|cell| cell.bg == red || cell.fg == red));
}

#[test]
fn zero_sized_render_area_does_not_touch_the_buffer() {
    let mut protocol = Picker::halfblocks().new_resize_protocol(image());
    let area = Rect::new(0, 0, CELL_WIDTH, CELL_HEIGHT);
    let mut buffer = Buffer::empty(area);
    let before = buffer.clone();
    StatefulImage::default().render(Rect::default(), &mut buffer, &mut protocol);
    assert_eq!(buffer, before);
}

#[test]
fn toml_round_trip_preserves_nested_configuration() {
    let original = "[storage.s3]\nallow_http = false\nendpoint = 'https://example.invalid'\n";
    let parsed: toml::Value = toml::from_str(original).unwrap();
    let encoded = toml::to_string_pretty(&parsed).unwrap();
    assert_eq!(toml::from_str::<toml::Value>(&encoded).unwrap(), parsed);
}

#[test]
fn toml_rejects_duplicate_keys_and_malformed_input() {
    assert!(toml::from_str::<toml::Value>("value=1\nvalue=0").is_err());
    assert!(toml::from_str::<toml::Value>("[invalid").is_err());
}

#[test]
fn executable_lookup_accepts_current_program_and_rejects_missing_path() {
    assert!(which::which(std::env::current_exe().unwrap()).is_ok());
    let root = tempfile::tempdir().unwrap();
    assert!(which::which(root.path().join("missing-program")).is_err());
}

#[test]
fn random_index_remains_within_the_requested_range() {
    const TRACK_COUNT: usize = 7;
    const SAMPLE_COUNT: usize = 64;
    for _ in 0..SAMPLE_COUNT {
        assert!(rand::random_range(0..TRACK_COUNT) < TRACK_COUNT);
    }
    assert_eq!(rand::random_range(0..1), 0);
}
