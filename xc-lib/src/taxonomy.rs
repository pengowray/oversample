use crate::types::{XcGroupTaxonomy, XcSearchResult, XcSpecies};
use std::collections::HashMap;

/// Builds a group's species list from search result pages.
///
/// Holds no network code, so the WASM frontend can use it with pages it
/// fetched itself. Call [`add_page`](Self::add_page) for each page of a
/// [`group_query`] search, then [`finish`](Self::finish).
pub struct SpeciesAccumulator {
    group: String,
    country: Option<String>,
    /// (genus, species epithet) -> (English name, recording count)
    species_map: HashMap<(String, String), (String, u32)>,
    total_recordings: u32,
}

impl SpeciesAccumulator {
    pub fn new(group: &str, country: Option<&str>) -> Self {
        Self {
            group: group.to_string(),
            country: country.map(|s| s.to_string()),
            species_map: HashMap::new(),
            total_recordings: 0,
        }
    }

    /// Count each recording on this page under its species.
    ///
    /// The English name comes from the first recording seen for a species.
    /// The total recording count comes from the latest page added.
    pub fn add_page(&mut self, page: &XcSearchResult) {
        self.total_recordings = page.num_recordings;
        for rec in &page.recordings {
            let key = (rec.genus.clone(), rec.sp.clone());
            let entry = self
                .species_map
                .entry(key)
                .or_insert_with(|| (rec.en.clone(), 0));
            entry.1 += 1;
        }
    }

    /// Build the taxonomy, sorted by English name (case-insensitive).
    ///
    /// `last_updated` is an RFC 3339 UTC timestamp such as
    /// `2026-11-09T12:00:00Z`. The caller supplies it because reading the
    /// clock works differently on desktop and in the browser.
    pub fn finish(self, last_updated: String) -> XcGroupTaxonomy {
        let mut species: Vec<XcSpecies> = self
            .species_map
            .into_iter()
            .map(|((genus, sp), (en, count))| XcSpecies {
                genus,
                sp,
                en,
                fam: String::new(),
                recording_count: count,
            })
            .collect();

        species.sort_by(|a, b| a.en.to_lowercase().cmp(&b.en.to_lowercase()));

        XcGroupTaxonomy {
            group: self.group,
            country: self.country,
            species,
            total_recordings: self.total_recordings,
            last_updated,
        }
    }
}

/// The search query that lists every recording in a group, optionally
/// limited to one country.
pub fn group_query(group: &str, country: Option<&str>) -> String {
    let mut query = format!("grp:{group}");
    if let Some(cnt) = country {
        query.push_str(&format!(" cnt:\"{cnt}\""));
    }
    query
}

/// Build a species list for a group by paginating through all API results.
///
/// Waits 1 second between page requests, since Xeno-Canto asks clients to
/// send at most 1 request per second.
///
/// The `on_progress` callback receives `(pages_fetched, total_pages)`.
#[cfg(feature = "http")]
pub async fn build_species_list<F>(
    client: &reqwest::Client,
    api_key: &str,
    group: &str,
    country: Option<&str>,
    mut on_progress: F,
) -> Result<XcGroupTaxonomy, String>
where
    F: FnMut(u32, u32),
{
    let query = group_query(group, country);
    let mut acc = SpeciesAccumulator::new(group, country);

    let per_page = 500;
    let mut page = 1u32;

    loop {
        let result = crate::api::search(client, api_key, &query, page, per_page).await?;
        let total_pages = result.num_pages;
        acc.add_page(&result);
        on_progress(page, total_pages);

        if page >= total_pages {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        page += 1;
    }

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    Ok(acc.finish(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::XcRecording;

    fn rec(genus: &str, sp: &str, en: &str) -> XcRecording {
        let mut r = crate::api::parse_recording(&serde_json::json!({
            "id": "1", "file": "https://x/a.mp3"
        }))
        .unwrap();
        r.genus = genus.into();
        r.sp = sp.into();
        r.en = en.into();
        r
    }

    fn page(num_recordings: u32, recordings: Vec<XcRecording>) -> XcSearchResult {
        XcSearchResult {
            num_recordings,
            num_species: 0,
            num_pages: 2,
            page: 1,
            recordings,
        }
    }

    #[test]
    fn group_query_adds_quoted_country() {
        assert_eq!(group_query("bats", None), "grp:bats");
        assert_eq!(
            group_query("bats", Some("United Kingdom")),
            "grp:bats cnt:\"United Kingdom\""
        );
    }

    #[test]
    fn accumulator_counts_across_pages_and_sorts() {
        let mut acc = SpeciesAccumulator::new("bats", Some("Australia"));
        acc.add_page(&page(
            3,
            vec![
                rec("Myotis", "daubentonii", "Daubenton's Bat"),
                rec("Pipistrellus", "pipistrellus", "common pipistrelle"),
            ],
        ));
        acc.add_page(&page(
            4,
            vec![
                // Same species with a different English name: the first one is kept.
                rec("Myotis", "daubentonii", "Water Bat"),
                rec("Nyctalus", "noctula", "Noctule"),
            ],
        ));
        let tax = acc.finish("2026-11-09T12:00:00Z".into());

        assert_eq!(tax.group, "bats");
        assert_eq!(tax.country.as_deref(), Some("Australia"));
        assert_eq!(tax.last_updated, "2026-11-09T12:00:00Z");
        assert_eq!(tax.total_recordings, 4, "total comes from the latest page");

        // A case-sensitive sort would put "common pipistrelle" last.
        let names: Vec<&str> = tax.species.iter().map(|s| s.en.as_str()).collect();
        assert_eq!(names, ["common pipistrelle", "Daubenton's Bat", "Noctule"]);

        let daub = &tax.species[1];
        assert_eq!(
            (daub.genus.as_str(), daub.sp.as_str()),
            ("Myotis", "daubentonii")
        );
        assert_eq!(daub.recording_count, 2);
        assert!(daub.fam.is_empty());
        assert_eq!(tax.species[0].recording_count, 1);
    }

    #[test]
    fn accumulator_with_no_pages_is_empty() {
        let tax = SpeciesAccumulator::new("frogs", None).finish("t".into());
        assert!(tax.species.is_empty());
        assert_eq!(tax.total_recordings, 0);
        assert_eq!(tax.country, None);
    }
}
