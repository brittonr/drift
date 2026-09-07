//! Select the same track that the filtered search view displays.
use super::{SearchResults, ServiceType, Track};

impl SearchResults {
    pub fn selected_track(&self, service: Option<ServiceType>, index: usize) -> Option<&Track> {
        self.tracks
            .iter()
            .filter(|track| service.is_none_or(|selected| selected == track.service))
            .nth(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::CoverArt;

    fn track(id: &str, service: ServiceType) -> Track {
        Track {
            id: id.into(),
            title: id.into(),
            artist: String::new(),
            album: String::new(),
            duration_seconds: 0,
            cover_art: CoverArt::None,
            service,
        }
    }
    fn results() -> SearchResults {
        SearchResults {
            tracks: vec![
                track("tidal", ServiceType::Tidal),
                track("youtube-a", ServiceType::YouTube),
                track("youtube-b", ServiceType::YouTube),
            ],
            ..Default::default()
        }
    }
    #[test]
    fn filtered_indices_follow_visible_rows() {
        let results = results();
        assert_eq!(
            results
                .selected_track(Some(ServiceType::YouTube), 0)
                .unwrap()
                .id,
            "youtube-a"
        );
        assert_eq!(
            results
                .selected_track(Some(ServiceType::YouTube), 1)
                .unwrap()
                .id,
            "youtube-b"
        );
        assert_eq!(
            results
                .selected_track(Some(ServiceType::Tidal), 0)
                .unwrap()
                .id,
            "tidal"
        );
    }
    #[test]
    fn unfiltered_indices_keep_original_order() {
        assert_eq!(results().selected_track(None, 0).unwrap().id, "tidal");
        assert_eq!(results().selected_track(None, 1).unwrap().id, "youtube-a");
    }
    #[test]
    fn empty_missing_and_out_of_range_selections_are_none() {
        assert!(SearchResults::default().selected_track(None, 0).is_none());
        assert!(results()
            .selected_track(Some(ServiceType::Bandcamp), 0)
            .is_none());
        assert!(results()
            .selected_track(Some(ServiceType::Tidal), 1)
            .is_none());
        assert!(results().selected_track(None, usize::MAX).is_none());
    }
}
