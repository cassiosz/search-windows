//! Tabs left alone give their page back (upstream Sleep.swift). This is only
//! the decision: which tabs are due, as a pure function of the clock. Taking
//! the page away, and asking it first whether it holds something typed, is
//! the app's business.

use crate::{Tab, Tabs};

/// How soon tabs sleep, as chosen in Settings (Sleep.swift `SleepPace`).
/// Three levels rather than numbers: what someone decides is how much a
/// waking tab bothers them, not how many minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SleepPace {
    Later,
    #[default]
    Normal,
    Sooner,
}

impl SleepPace {
    /// Away this long, a tab sleeps: two hours, half an hour, ten minutes.
    pub fn wait_ms(self) -> u64 {
        match self {
            SleepPace::Later => 2 * 60 * 60 * 1000,
            SleepPace::Normal => 30 * 60 * 1000,
            SleepPace::Sooner => 10 * 60 * 1000,
        }
    }
}

/// When memory is short the wait shrinks, whatever was chosen: five minutes
/// on a warning, nothing when critical (Sleep.swift `watchForSleep`).
pub const WAIT_ON_PRESSURE_WARNING_MS: u64 = 5 * 60 * 1000;
pub const WAIT_ON_PRESSURE_CRITICAL_MS: u64 = 0;

impl Tabs {
    /// Why a tab has to stay awake, None when nothing keeps it. The clock is
    /// the caller's business; this is everything else (Browser.awake(because:)).
    ///
    /// `shut`: the last window is closed with the app running, so nothing is
    /// on screen and no tab is "in front" (Sleep.swift `sleepShut`).
    pub fn awake_because(&self, tab: &Tab, shut: bool) -> Option<&'static str> {
        if !shut && tab.id == self.selected_id() {
            return Some("on screen");
        }
        // Pinned tabs are put down by hand, with Ctrl+W. With no window open
        // a pin stays awake only while its site may send notifications.
        if tab.pinned {
            if !shut {
                return Some("pinned");
            }
            if tab.notifies {
                return Some("pinned, and its site may send notifications");
            }
        }
        if tab.is_blank() {
            return Some("blank");
        }
        if tab.asleep {
            return Some("already asleep");
        }
        if tab.loading {
            return Some("still loading");
        }
        if tab.keep_awake {
            return Some("kept awake for this site");
        }
        if tab.audible {
            return Some("playing sound");
        }
        if tab.busy {
            return Some("busy");
        }
        None
    }

    /// Every tab gone `wait_ms` without being looked at that nothing keeps
    /// awake, the one left longest first (Browser.sleepIdle).
    pub fn due_to_sleep(&self, now_ms: u64, wait_ms: u64) -> Vec<u64> {
        let mut due: Vec<&Tab> = self
            .iter()
            .filter(|t| now_ms.saturating_sub(t.last_active_ms) >= wait_ms && self.awake_because(t, false).is_none())
            .collect();
        // Stable, so tabs left at the same moment keep the row's order.
        due.sort_by_key(|t| t.last_active_ms);
        due.into_iter().map(|t| t.id).collect()
    }

    /// With the last window closed, every tab the rules let go sleeps now
    /// rather than later (Browser.sleepShut).
    pub fn due_when_shut(&self) -> Vec<u64> {
        self.iter().filter(|t| self.awake_because(t, true).is_none()).map(|t| t.id).collect()
    }

    /// Tabs that close themselves (Closing.swift): left alone `wait_ms`, and
    /// not one you said to keep (pinned, named), nor the one on screen,
    /// blank, or busy as a tab kept awake is. Oldest first.
    pub fn due_to_close(&self, now_ms: u64, wait_ms: u64) -> Vec<u64> {
        let stays = |t: &Tab| {
            t.id == self.selected_id()
                || t.pinned
                || t.name.is_some()
                || t.is_blank()
                || t.keep_awake
                || t.audible
                || t.busy
        };
        let mut due: Vec<&Tab> =
            self.iter().filter(|t| now_ms.saturating_sub(t.last_active_ms) >= wait_ms && !stays(t)).collect();
        due.sort_by_key(|t| t.last_active_ms);
        due.into_iter().map(|t| t.id).collect()
    }

    /// The page was given back: the tab keeps its address and title, and
    /// wakes when selected. The tab in front never sleeps.
    pub fn mark_asleep(&mut self, id: u64) {
        if id != self.selected_id()
            && let Some(t) = self.get_mut(id)
        {
            t.asleep = true;
            // Its page is gone, and its sound with it: the next page says when it plays.
            t.audible = false;
        }
    }
}
