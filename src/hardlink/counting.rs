use super::{
    DeduplicateSharedSize, HardlinkAware, HardlinkList, RecordHardlinks, RecordHardlinksArgument,
    aware::ReportHardlinksError,
};
use crate::{
    data_tree::DataTree,
    os_string_display::OsStringDisplay,
    reporter::{Event, Reporter, event::HardlinkDetection},
    size,
};
use dashmap::{DashMap, mapref::entry::Entry};
use std::{convert::Infallible, fmt::Debug, os::unix::fs::MetadataExt, sync::Mutex};

/// Deduplicate hardlinks for the root only, without remembering link paths.
///
/// Memory is bounded by the inodes whose links have not all been seen yet,
/// instead of by every detected link path. Only valid when the output keeps
/// nothing below the root and reports no shared-link details.
#[derive(Debug)]
pub struct Counting<Size> {
    /// `(dev, ino)` → `(size, nlink, links seen)`, removed once all links were seen.
    pending: DashMap<(u64, u64), (Size, u64, u64)>,
    duplicated: Mutex<Size>,
}

pub use Counting as HardlinkCounting;

impl<Size: Default> Default for Counting<Size> {
    fn default() -> Self {
        Counting {
            pending: DashMap::new(),
            duplicated: Mutex::new(Size::default()),
        }
    }
}

impl<Size, Report> RecordHardlinks<Size, Report> for Counting<Size>
where
    Size: size::Size,
    Report: Reporter<Size> + ?Sized,
{
    type Error = Infallible;

    fn record_hardlinks(
        &self,
        argument: RecordHardlinksArgument<Size, Report>,
    ) -> Result<(), Self::Error> {
        let RecordHardlinksArgument {
            path,
            stats,
            size,
            reporter,
        } = argument;

        let links = stats.nlink();
        if stats.is_dir() || links <= 1 {
            return Ok(());
        }

        reporter.report(Event::DetectHardlink(HardlinkDetection {
            path,
            stats,
            size,
            links,
        }));

        match self.pending.entry((stats.dev(), stats.ino())) {
            Entry::Vacant(entry) => {
                entry.insert((size, links, 1));
            }
            Entry::Occupied(mut entry) => {
                let (recorded_size, recorded_links, seen) = entry.get_mut();
                // Same as `HardlinkList::add`: a changed size or link count is not deduplicated.
                if *recorded_size != size || *recorded_links != links {
                    return Ok(());
                }
                *seen += 1;
                *self.duplicated.lock().expect("lock duplicated size") += size;
                if *seen >= links {
                    entry.remove();
                }
            }
        }
        Ok(())
    }
}

impl<Size> DeduplicateSharedSize<Size> for Counting<Size>
where
    DataTree<OsStringDisplay, Size>: Send,
    Size: size::Size + Sync,
{
    type Report = ();
    type Error = Infallible;
    fn deduplicate(
        self,
        data_tree: &mut DataTree<OsStringDisplay, Size>,
    ) -> Result<(), Infallible> {
        let duplicated = self.duplicated.into_inner().expect("lock duplicated size");
        data_tree.subtract_root_size(duplicated);
        Ok(())
    }
}

/// [`HardlinkAware`] by default, [`HardlinkCounting`] when only the deduplicated root size is needed.
#[derive(Debug)]
pub enum AwareOrCounting<Size> {
    Aware(HardlinkAware<Size>),
    Counting(HardlinkCounting<Size>),
}

pub use AwareOrCounting as HardlinkAwareOrCounting;

impl<Size: Default> AwareOrCounting<Size> {
    pub fn new(root_only: bool) -> Self {
        if root_only {
            AwareOrCounting::Counting(HardlinkCounting::default())
        } else {
            AwareOrCounting::Aware(HardlinkAware::new())
        }
    }
}

impl<Size, Report> RecordHardlinks<Size, Report> for AwareOrCounting<Size>
where
    Size: size::Size + Eq + Debug,
    Report: Reporter<Size> + ?Sized,
{
    type Error = ReportHardlinksError<Size>;

    fn record_hardlinks(
        &self,
        argument: RecordHardlinksArgument<Size, Report>,
    ) -> Result<(), Self::Error> {
        match self {
            AwareOrCounting::Aware(aware) => aware.record_hardlinks(argument),
            AwareOrCounting::Counting(counting) => match counting.record_hardlinks(argument) {
                Ok(()) => Ok(()),
            },
        }
    }
}

impl<Size> DeduplicateSharedSize<Size> for AwareOrCounting<Size>
where
    DataTree<OsStringDisplay, Size>: Send,
    Size: size::Size + Sync,
{
    type Report = Option<HardlinkList<Size>>;
    type Error = Infallible;
    fn deduplicate(
        self,
        data_tree: &mut DataTree<OsStringDisplay, Size>,
    ) -> Result<Self::Report, Self::Error> {
        match self {
            AwareOrCounting::Aware(aware) => aware.deduplicate(data_tree).map(Some),
            AwareOrCounting::Counting(counting) => counting.deduplicate(data_tree).map(|()| None),
        }
    }
}
