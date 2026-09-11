//! Motion-page residency policy: which pages to copy, which to keep and which
//! to evict. Kept free of wgpu so the same decisions can be measured headlessly
//! by the S/M/L harness instead of being duplicated there.
use crate::pages::PageTable;
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    hash: u64,
    resident: bool,
    last_used: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageUpload {
    pub page: usize,
    pub slot_offset: usize,
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Pages to copy this frame, in admission order.
    pub uploads: Vec<PageUpload>,
    pub skipped: usize,
    pub evicted: usize,
    pub admitted: usize,
    pub omitted: usize,
    pub resident_pages: usize,
    pub resident_bytes: u64,
}

impl Plan {
    pub fn upload_bytes(&self) -> u64 {
        self.uploads.iter().map(|upload| upload.len as u64).sum()
    }
}

pub struct Pager {
    identity: u64,
    slots: Vec<Slot>,
    clock: u64,
    budget: u64,
    resident_bytes: u64,
}

impl Pager {
    pub fn new(budget: u64) -> Self {
        Self {
            identity: u64::MAX,
            slots: Vec::new(),
            clock: 0,
            budget,
            resident_bytes: 0,
        }
    }
    pub fn budget(&self) -> u64 {
        self.budget
    }
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
    }
    pub fn resident_bytes(&self) -> u64 {
        self.resident_bytes
    }
    pub fn forget(&mut self) {
        for slot in &mut self.slots {
            *slot = Slot::default();
        }
    }
    pub fn is_resident(&self, page: usize) -> bool {
        self.slots.get(page).is_some_and(|slot| slot.resident)
    }

    /// `required` lists the pages the display needs, nearest the playhead
    /// first. The budget admits that order; pages that do not fit are reported
    /// as omitted rather than silently pretending to be drawn.
    pub fn plan(
        &mut self,
        identity: u64,
        table: &PageTable,
        hashes: &[Option<u64>],
        required: &[usize],
        page_len: impl Fn(usize) -> Range<usize>,
    ) -> Plan {
        if self.identity != identity || self.slots.len() != table.page_count() {
            self.identity = identity;
            self.slots = vec![Slot::default(); table.page_count()];
        }
        self.clock += 1;
        let mut plan = Plan::default();
        let mut used = 0_u64;
        for (order, page) in required.iter().enumerate() {
            if *page >= self.slots.len() {
                continue;
            }
            let len = page_len(*page).len() as u64;
            if plan.admitted > 0 && used + len > self.budget {
                plan.omitted += 1;
                continue;
            }
            used += len;
            plan.admitted += 1;
            let hash = hashes.get(*page).copied().flatten();
            let slot = self.slots[*page];
            let unchanged = slot.resident
                && match hash {
                    Some(hash) => slot.hash == hash,
                    // An unknown fingerprint means the payload identity has not
                    // changed, so a resident page is still current.
                    None => slot.hash == 0,
                };
            if unchanged {
                plan.skipped += 1;
                self.slots[*page].last_used = self.clock + order as u64;
                continue;
            }
            plan.uploads.push(PageUpload {
                page: *page,
                slot_offset: table.slot_of(*page),
                offset: page_len(*page).start,
                len: page_len(*page).len(),
            });
            self.slots[*page] = Slot {
                hash: hash.unwrap_or(0),
                resident: true,
                last_used: self.clock + order as u64,
            };
        }
        let mut resident_bytes: u64 = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.resident)
            .map(|(page, _)| page_len(page).len() as u64)
            .sum();
        if resident_bytes > self.budget {
            let mut candidates: Vec<(u64, usize)> = self
                .slots
                .iter()
                .enumerate()
                .filter(|(page, slot)| slot.resident && !required.contains(page))
                .map(|(page, slot)| (slot.last_used, page))
                .collect();
            candidates.sort_unstable();
            for (_, page) in candidates {
                if resident_bytes <= self.budget {
                    break;
                }
                self.slots[page].resident = false;
                self.slots[page].hash = 0;
                resident_bytes -= page_len(page).len() as u64;
                plan.evicted += 1;
            }
        }
        plan.resident_pages = self.slots.iter().filter(|slot| slot.resident).count();
        plan.resident_bytes = resident_bytes;
        self.resident_bytes = resident_bytes;
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::{PAGE_BYTES, PageTable};

    fn table(motions: usize) -> PageTable {
        PageTable::new(0, motions * 2 * 28, motions).unwrap()
    }
    fn page_len(table: &PageTable) -> impl Fn(usize) -> Range<usize> + '_ {
        move |page| {
            let motions = table.motions_in(page);
            let start = motions.start * 2 * 28;
            start..start + (motions.end - motions.start) * 2 * 28
        }
    }

    #[test]
    fn first_plan_uploads_then_reload_transfers_nothing() {
        let table = table(20_000);
        let hashes: Vec<Option<u64>> = (0..table.page_count())
            .map(|p| Some(p as u64 + 1))
            .collect();
        let required: Vec<usize> = (0..table.page_count()).collect();
        let mut pager = Pager::new(u64::MAX);
        let first = pager.plan(7, &table, &hashes, &required, page_len(&table));
        assert_eq!(first.uploads.len(), table.page_count());
        assert!(first.upload_bytes() > 0);
        let reload = pager.plan(7, &table, &hashes, &required, page_len(&table));
        assert!(reload.uploads.is_empty());
        assert_eq!(reload.skipped, table.page_count());
        // A new payload identity always re-copies.
        let fresh = pager.plan(8, &table, &hashes, &required, page_len(&table));
        assert_eq!(fresh.uploads.len(), table.page_count());
    }

    #[test]
    fn only_pages_with_new_fingerprints_are_copied() {
        let table = table(20_000);
        let mut hashes: Vec<Option<u64>> =
            (0..table.page_count()).map(|p| Some(p as u64)).collect();
        let required: Vec<usize> = (0..table.page_count()).collect();
        let mut pager = Pager::new(u64::MAX);
        pager.plan(1, &table, &hashes, &required, page_len(&table));
        hashes[1] = Some(999);
        let plan = pager.plan(1, &table, &hashes, &required, page_len(&table));
        assert_eq!(plan.uploads.len(), 1);
        assert_eq!(plan.uploads[0].page, 1);
        assert_eq!(plan.uploads[0].len, PAGE_BYTES.min(plan.uploads[0].len));
    }

    #[test]
    fn budget_admits_nearest_pages_and_reports_omissions() {
        let table = table(100_000);
        let hashes: Vec<Option<u64>> = (0..table.page_count()).map(|_| Some(1)).collect();
        let required: Vec<usize> = (0..table.page_count()).collect();
        let mut pager = Pager::new((PAGE_BYTES * 3) as u64);
        let plan = pager.plan(1, &table, &hashes, &required, page_len(&table));
        assert_eq!(plan.admitted, 3);
        assert_eq!(plan.omitted, table.page_count() - 3);
        assert!(plan.resident_bytes <= (PAGE_BYTES * 3) as u64);

        // Scrolling back keeps the resident pages and copies nothing new.
        let subset: Vec<usize> = (0..3).collect();
        let revisit = pager.plan(1, &table, &hashes, &subset, page_len(&table));
        assert!(revisit.uploads.is_empty());
    }
}
