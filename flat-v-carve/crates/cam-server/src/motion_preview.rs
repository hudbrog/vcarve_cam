//! Exact recorded motions, paged from a service-owned file instead of truncated.
use cam_core::motion::Motion;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Seek, Write},
    path::Path,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub const PAGE_MOTIONS: usize = 20_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub offset: u64,
    pub bytes: usize,
}

pub fn write(path: &Path, motions: &[Motion]) -> io::Result<Vec<Page>> {
    let mut file = BufWriter::new(OpenOptions::new().write(true).truncate(true).open(path)?);
    let mut pages = Vec::new();
    for chunk in motions.chunks(PAGE_MOTIONS) {
        let offset = file.stream_position()?;
        serde_json::to_writer(&mut file, chunk)?;
        let bytes = (file.stream_position()? - offset) as usize;
        pages.push(Page { offset, bytes });
    }
    file.flush()?;
    Ok(pages)
}

pub async fn read(path: &Path, page: &Page) -> io::Result<Vec<Motion>> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(io::SeekFrom::Start(page.offset)).await?;
    let mut bytes = vec![0; page.bytes];
    file.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_core::motion::{MotionKind, Position};

    #[tokio::test]
    async fn all_motions_survive_page_boundaries_and_file_leases() {
        let file = crate::artifact::PlanFile::create().unwrap();
        let path = file.path().to_owned();
        let motions: Vec<_> = (0..PAGE_MOTIONS * 2 + 3)
            .map(|id| Motion {
                id,
                tool_id: if id < PAGE_MOTIONS { "endmill" } else { "vbit" }.into(),
                operation_id: "carve".into(),
                layer: id / PAGE_MOTIONS,
                kind: if id % 2 == 0 {
                    MotionKind::Cut
                } else {
                    MotionKind::RapidXY
                },
                start: Position {
                    x: id as f64,
                    y: 2.,
                    z: -1.,
                },
                end: Position {
                    x: id as f64 + 0.001,
                    y: 3.,
                    z: -2.,
                },
                feed_mm_min: Some(100.),
            })
            .collect();
        let pages = write(&path, &motions).unwrap();
        assert_eq!(pages.len(), 3);
        let lease = file.clone();
        drop(file);
        let mut restored = Vec::new();
        for page in &pages {
            let chunk = read(&path, page).await.unwrap();
            assert!(chunk.len() <= PAGE_MOTIONS);
            restored.extend(chunk);
        }
        assert_eq!(restored, motions);
        drop(lease);
        assert!(!path.exists());
    }

    #[test]
    fn empty_plan_has_no_pages() {
        let file = crate::artifact::PlanFile::create().unwrap();
        assert!(write(file.path(), &[]).unwrap().is_empty());
    }
}
