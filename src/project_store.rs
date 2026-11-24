//! Projects (.batproj) in the browser's private storage (OPFS).
//!
//! A project lives in one store (`cloud::store`): the device store, or the
//! signed-in Google account's cache. Its home is noted when it is loaded or
//! saved, and later saves go there. While signed in, the project list shows
//! the account's projects and the device store's ones, so nothing made while
//! signed out is hidden.

use oversample_core::erased_sort::SortErased;
use crate::cloud::store::{self, Store};
use crate::project::BatProject;

/// Storage key for a project file.
fn project_key(id: &str) -> String {
    format!("{}.batproj", id)
}

/// The stores to look in, in order: the active one, then (when signed in)
/// the device store.
fn stores() -> Vec<Store> {
    let active = store::active();
    if active == Store::Device {
        vec![Store::Device]
    } else {
        vec![active, Store::Device]
    }
}

/// Save a project to its home as YAML, then hand it to the Google Drive copy
/// (only a project in the signed-in account's cache goes there).
pub async fn save_project(project: &BatProject) -> Result<(), String> {
    write_project_local(project).await?;
    if matches!(store::project_home(&project.id), Store::Account(_)) {
        crate::cloud::sync::project_saved(project);
    }
    Ok(())
}

/// Save a project to its home as YAML, without telling the Google Drive copy
/// (for a project that came from Drive). Returns the YAML written.
pub async fn write_project_local(project: &BatProject) -> Result<String, String> {
    let yaml = yaml_serde::to_string(project).map_err(|e| format!("YAML serialize: {e}"))?;
    let home = store::project_home(&project.id);
    store::set_project_home(&project.id, home.clone());
    write_project_in(&home, &project.id, &yaml).await?;
    Ok(yaml)
}

/// Write project `id` into `store` as `yaml`.
pub async fn write_project_in(store: &Store, id: &str, yaml: &str) -> Result<(), String> {
    store::write(&store.projects_dir(), &project_key(id), yaml).await
}

/// Ids of the projects in `store`.
pub async fn project_ids_in(store: &Store) -> Vec<String> {
    store::keys(&store.projects_dir(), ".batproj")
        .await
        .into_iter()
        .filter_map(|n| n.strip_suffix(".batproj").map(str::to_string))
        .collect()
}

/// Read project `id` from `store`.
pub async fn load_project_in(store: &Store, id: &str) -> Result<Option<BatProject>, String> {
    let Some(text) = store::read(&store.projects_dir(), &project_key(id)).await else {
        return Ok(None);
    };
    let project: BatProject =
        yaml_serde::from_str(&text).map_err(|e| format!("YAML deserialize: {e}"))?;
    if project.version > crate::project::PROJECT_FORMAT_VERSION {
        log::warn!(
            "Project {id} is format version {} but this app supports {}; loading \
             best-effort — some data may be missing or misread.",
            project.version,
            crate::project::PROJECT_FORMAT_VERSION
        );
    }
    Ok(Some(project))
}

/// Load a project by its ID (the account's cache first, then the device
/// store), and note its home. Returns None if not found.
pub async fn load_project(id: &str) -> Result<Option<BatProject>, String> {
    for st in stores() {
        if let Some(p) = load_project_in(&st, id).await? {
            store::set_project_home(id, st);
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// Summary info for a saved project (for the list picker).
#[derive(Clone)]
pub struct ProjectSummary {
    pub id: String,
    pub name: Option<String>,
    pub file_count: usize,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
}

/// Every project in the active store and (when signed in) the device store,
/// most recently changed first.
pub async fn list_projects() -> Result<Vec<ProjectSummary>, String> {
    let mut result: Vec<ProjectSummary> = Vec::new();
    for st in stores() {
        for id in project_ids_in(&st).await {
            if result.iter().any(|r| r.id == id) {
                continue;
            }
            match load_project_in(&st, &id).await {
                Ok(Some(proj)) => result.push(ProjectSummary {
                    id,
                    name: proj.name,
                    file_count: proj.files.len(),
                    created_at: proj.created_at,
                    modified_at: proj.modified_at,
                }),
                _ => result.push(ProjectSummary {
                    id,
                    name: None,
                    file_count: 0,
                    created_at: None,
                    modified_at: None,
                }),
            }
        }
    }

    // Sort by modified_at descending (most recent first). modified_at is ISO-8601,
    // which is lexically sortable in chronological order, so a plain string compare
    // is correct; projects with no timestamp (None < Some) consistently sort last.
    result.sort_erased_by(|a, b| b.modified_at.cmp(&a.modified_at));

    Ok(result)
}

/// Delete a project by ID from the store that holds it.
pub async fn delete_project(id: &str) -> Result<(), String> {
    let mut home = None;
    for st in stores() {
        if store::read(&st.projects_dir(), &project_key(id))
            .await
            .is_some()
        {
            home = Some(st);
            break;
        }
    }
    let home = home.unwrap_or_else(|| store::project_home(id));
    store::remove(&home.projects_dir(), &project_key(id)).await?;
    // A project of the account: its Google Drive copy goes to the Drive
    // trash, or it would come back.
    if matches!(home, Store::Account(_)) {
        crate::cloud::sync::project_deleted(id);
    }
    Ok(())
}

/// Export a project as a YAML string (for download).
pub fn export_project_yaml(project: &BatProject) -> Result<String, String> {
    yaml_serde::to_string(project).map_err(|e| format!("YAML serialize: {e}"))
}
