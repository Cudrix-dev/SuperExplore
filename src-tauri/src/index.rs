use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap};
use serde::Serialize;
use std::{
    collections::BinaryHeap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Instant, UNIX_EPOCH},
};

pub const LIMIT: usize = 100;
const NTFS_ROOT_RECORD: u64 = 5;
const NTFS_RECORD_NUMBER_MASK: u64 = (1 << 48) - 1;

pub struct Entry {
    pub parent: u64,
    pub name: Box<str>,
    folded: Option<Box<str>>,
    pub directory: bool,
}

impl Entry {
    pub fn new(parent: u64, name: String, directory: bool) -> Self {
        let lowercase = name.to_lowercase();
        let folded = (lowercase != name).then(|| lowercase.into_boxed_str());
        Self {
            parent,
            name: name.into_boxed_str(),
            folded,
            directory,
        }
    }
}

pub struct Volume {
    pub root: PathBuf,
    pub entries: hashbrown::HashMap<u64, Entry, FxBuildHasher>,
}

impl Volume {
    fn path(&self, mut id: u64) -> Option<PathBuf> {
        let mut components = Vec::new();
        while id & NTFS_RECORD_NUMBER_MASK != NTFS_ROOT_RECORD {
            let entry = self.entries.get(&id)?;
            if entry.parent == id || components.len() >= 1024 {
                return None;
            }
            components.push(entry.name.as_ref());
            id = entry.parent;
        }

        let mut path = self.root.clone();
        path.extend(components.into_iter().rev());
        Some(path)
    }
}

#[derive(Clone, Default)]
pub struct Index(Arc<IndexState>);

#[derive(Default)]
struct IndexState {
    volumes: RwLock<FxHashMap<String, Arc<RwLock<Volume>>>>,
    warning: RwLock<Option<String>>,
    ready: AtomicBool,
    revision: AtomicU64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    ready: bool,
    entries: usize,
    volumes: usize,
    revision: u64,
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    name: String,
    path: String,
    directory: bool,
    size: Option<u64>,
    modified_ms: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    hits: Vec<Hit>,
    elapsed_ms: f64,
}

// Lower scores rank first. The largest item is removed when the heap is full.
type Match = (u8, usize, u64);

fn keep_match(heap: &mut BinaryHeap<Match>, candidate: Match) {
    if heap.len() < LIMIT {
        heap.push(candidate);
    } else if heap.peek().is_some_and(|worst| candidate < *worst) {
        *heap.peek_mut().unwrap() = candidate;
    }
}

impl Index {
    pub fn set_volume(&self, id: String, volume: Volume) -> Arc<RwLock<Volume>> {
        let volume = Arc::new(RwLock::new(volume));
        self.0.volumes.write().unwrap().insert(id, volume.clone());
        self.changed();
        volume
    }

    pub fn retain_volumes(&self, keep: impl Fn(&str) -> bool) {
        let mut volumes = self.0.volumes.write().unwrap();
        let count = volumes.len();
        volumes.retain(|id, _| keep(id));
        if count != volumes.len() {
            self.changed();
        }
    }

    pub fn changed(&self) {
        self.0.revision.fetch_add(1, Ordering::Relaxed);
    }

    pub fn ready(&self) {
        self.0.ready.store(true, Ordering::Release);
    }

    pub fn set_warning(&self, warning: Option<String>) {
        *self.0.warning.write().unwrap() = warning;
    }

    pub fn status(&self) -> Status {
        let ready = self.0.ready.load(Ordering::Acquire);
        let volumes = self.0.volumes.read().unwrap();
        Status {
            ready,
            entries: volumes
                .values()
                .map(|v| v.read().unwrap().entries.len())
                .sum(),
            volumes: volumes.len(),
            revision: self.0.revision.load(Ordering::Relaxed),
            warning: self.0.warning.read().unwrap().clone(),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.0.ready.load(Ordering::Acquire)
    }

    pub fn search(&self, query: &str) -> SearchResponse {
        let start = Instant::now();
        let query = query.trim().to_lowercase();
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut hits = Vec::new();
        if !terms.is_empty() {
            let volumes = {
                let volumes = self.0.volumes.read().unwrap();
                let mut volumes: Vec<_> = volumes
                    .iter()
                    .map(|(id, v)| (id.clone(), v.clone()))
                    .collect();
                volumes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                volumes
            };
            // Keep each volume's entries and parent links stable throughout this search.
            let volumes: Vec<_> = volumes.iter().map(|(_, v)| v.read().unwrap()).collect();
            let terms = &terms;
            let query = query.as_str();
            let matches = volumes
                .par_iter()
                .enumerate()
                .flat_map(|(volume, data)| {
                    data.entries.par_iter().filter_map(move |(&id, entry)| {
                        let candidate = entry.folded.as_deref().unwrap_or(&entry.name);
                        if id & NTFS_RECORD_NUMBER_MASK == NTFS_ROOT_RECORD
                            || !terms.iter().all(|term| candidate.contains(term))
                        {
                            return None;
                        }
                        let rank = if candidate == query {
                            0
                        } else if candidate.starts_with(query) {
                            1
                        } else {
                            2
                        };
                        Some((rank, volume, id))
                    })
                })
                .fold(BinaryHeap::new, |mut heap, candidate| {
                    keep_match(&mut heap, candidate);
                    heap
                })
                .reduce(BinaryHeap::new, |mut heap, other| {
                    for candidate in other {
                        keep_match(&mut heap, candidate);
                    }
                    heap
                });

            for (_, volume, id) in matches.into_sorted_vec() {
                if let Some(path) = volumes[volume].path(id) {
                    let entry = &volumes[volume].entries[&id];
                    hits.push((entry.name.to_string(), path, entry.directory));
                }
            }
        }

        let hits = hits
            .into_par_iter()
            .map(|(name, path, directory)| {
                let metadata = fs::symlink_metadata(&path).ok();
                let modified_ms = metadata
                    .as_ref()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64);
                let size = metadata
                    .filter(|_| !directory)
                    .map(|metadata| metadata.len());
                Hit {
                    name,
                    path: path.to_string_lossy().into_owned(),
                    directory,
                    size,
                    modified_ms,
                }
            })
            .collect();
        SearchResponse {
            hits,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_caps_results_and_resolves_renamed_ancestors() {
        let mut entries = hashbrown::HashMap::default();
        entries.insert(7, Entry::new(5, "Before".into(), true));
        for id in 100..240 {
            entries.insert(id, Entry::new(7, format!("Résumé Plan {id}.txt"), false));
        }
        entries.insert(240, Entry::new(7, "Unrelated.txt".into(), false));
        let index = Index::default();
        let volume = index.set_volume(
            "test".into(),
            Volume {
                root: PathBuf::from(r"X:\"),
                entries,
            },
        );
        let result = index.search("  PLAN   RÉSUMÉ ");
        assert_eq!(result.hits.len(), LIMIT);
        assert_eq!(result.hits[0].path, r"X:\Before\Résumé Plan 100.txt");
        volume
            .write()
            .unwrap()
            .entries
            .insert(7, Entry::new(5, "After".into(), true));
        assert_eq!(
            index.search("plan").hits[0].path,
            r"X:\After\Résumé Plan 100.txt"
        );
    }
}
