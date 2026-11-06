//! Tab grouping for the workspace: at most two side-by-side groups. Pure data,
//! so drag, merge and close rules are testable without a window.

pub type TabId = u64;

/// Where a dragged tab would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// Insert into a tab strip before `index`.
    Strip { group: usize, index: usize },
    /// Append to the other visible group.
    Pane { group: usize },
    /// Open the right-hand group with this tab.
    Split,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    groups: [Vec<TabId>; 2],
    selected: [Option<TabId>; 2],
    split: bool,
}

impl Layout {
    pub fn tabs(&self, group: usize) -> &[TabId] {
        &self.groups[group]
    }

    pub fn split(&self) -> bool {
        self.split
    }

    pub fn len(&self) -> usize {
        self.groups[0].len() + self.groups[1].len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Left group first, then right: the persisted and Ctrl+Tab order.
    pub fn order(&self) -> Vec<TabId> {
        self.groups.concat()
    }

    pub fn group_of(&self, tab: TabId) -> Option<usize> {
        self.groups.iter().position(|g| g.contains(&tab))
    }

    pub fn selected(&self, group: usize) -> Option<TabId> {
        self.selected[group]
    }

    pub fn is_visible(&self, tab: TabId) -> bool {
        self.group_of(tab).is_some_and(|g| (g == 0 || self.split) && self.selected[g] == Some(tab))
    }

    pub fn add(&mut self, tab: TabId, group: usize, index: Option<usize>) {
        let tabs = &mut self.groups[group];
        tabs.insert(index.unwrap_or(tabs.len()).min(tabs.len()), tab);
        self.selected[group].get_or_insert(tab);
    }

    pub fn select(&mut self, tab: TabId) {
        if let Some(group) = self.group_of(tab) {
            self.selected[group] = Some(tab);
            if group == 1 {
                self.split = true;
            }
        }
    }

    /// Removes a tab; a removed selection passes to its neighbour.
    pub fn remove(&mut self, tab: TabId) -> Option<usize> {
        let group = self.group_of(tab)?;
        let tabs = &mut self.groups[group];
        let index = tabs.iter().position(|t| *t == tab).unwrap();
        tabs.remove(index);
        if self.selected[group] == Some(tab) {
            self.selected[group] = tabs.get(index.min(tabs.len().saturating_sub(1))).copied();
        }
        Some(group)
    }

    pub fn move_tab(&mut self, tab: TabId, group: usize, index: Option<usize>) {
        let Some(old) = self.group_of(tab) else { return };
        if old == group {
            let Some(mut index) = index else { return };
            let tabs = &mut self.groups[group];
            let position = tabs.iter().position(|t| *t == tab).unwrap();
            tabs.remove(position);
            if position < index {
                index -= 1;
            }
            tabs.insert(index.min(tabs.len()), tab);
            return;
        }
        self.remove(tab);
        self.add(tab, group, index);
        self.selected[group] = Some(tab);
    }

    pub fn show_split(&mut self) {
        self.split = true;
    }

    /// An empty group never stays on screen; the window size is unaffected.
    pub fn collapse_empty_group(&mut self) {
        if !self.split {
            return;
        }
        if self.groups[0].is_empty() {
            self.groups.swap(0, 1);
            self.selected.swap(0, 1);
        }
        if self.groups[1].is_empty() {
            self.split = false;
            self.selected[1] = None;
        }
    }

    /// Whether `target` is a meaningful destination for `tab`.
    pub fn accepts(&self, tab: TabId, target: DropTarget) -> bool {
        let Some(group) = self.group_of(tab) else { return false };
        match target {
            DropTarget::Strip { group: g, .. } => g == 0 || self.split,
            DropTarget::Pane { group: g } => self.split && g != group,
            DropTarget::Split => !self.split && self.len() > 1,
        }
    }

    /// Applies a drop, collapsing an emptied group. Returns whether it applied.
    pub fn drop_tab(&mut self, tab: TabId, target: DropTarget) -> bool {
        if !self.accepts(tab, target) {
            return false;
        }
        match target {
            DropTarget::Strip { group, index } => self.move_tab(tab, group, Some(index)),
            DropTarget::Pane { group } => self.move_tab(tab, group, None),
            DropTarget::Split => {
                self.show_split();
                self.move_tab(tab, 1, Some(0));
            }
        }
        self.collapse_empty_group();
        self.select(tab);
        true
    }

    /// Restores a saved arrangement; an unusable split collapses.
    pub fn restore(tabs: impl IntoIterator<Item = (TabId, usize)>, split: bool) -> Self {
        let mut layout = Layout::default();
        for (tab, group) in tabs {
            layout.add(tab, if split { group.min(1) } else { 0 }, None);
        }
        layout.split = split;
        layout.collapse_empty_group();
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(tabs: &[TabId]) -> Layout {
        Layout::restore(tabs.iter().map(|t| (*t, 0)), false)
    }

    #[test]
    fn split_then_merge_back_collapses() {
        let mut l = layout(&[1, 2]);
        assert!(l.drop_tab(1, DropTarget::Split));
        assert!(l.split());
        assert_eq!((l.tabs(0), l.tabs(1)), (&[2][..], &[1][..]));
        assert!(l.is_visible(1) && l.is_visible(2));
        assert!(l.drop_tab(1, DropTarget::Strip { group: 0, index: 0 }));
        assert!(!l.split());
        assert_eq!(l.tabs(0), [1, 2]);
    }

    #[test]
    fn only_tab_cannot_split_and_own_pane_is_not_a_target() {
        let mut l = layout(&[1]);
        assert!(!l.drop_tab(1, DropTarget::Split));
        let mut l = layout(&[1, 2, 3]);
        l.drop_tab(3, DropTarget::Split);
        assert!(!l.accepts(3, DropTarget::Pane { group: 1 }));
        assert!(l.accepts(1, DropTarget::Pane { group: 1 }));
        l.drop_tab(1, DropTarget::Pane { group: 1 });
        assert_eq!(l.tabs(1), [3, 1]);
    }

    #[test]
    fn reorder_within_strip() {
        let mut l = layout(&[1, 2, 3]);
        l.drop_tab(3, DropTarget::Strip { group: 0, index: 0 });
        assert_eq!(l.order(), [3, 1, 2]);
        l.drop_tab(3, DropTarget::Strip { group: 0, index: 2 });
        assert_eq!(l.order(), [1, 3, 2]);
        l.drop_tab(1, DropTarget::Strip { group: 0, index: 3 });
        assert_eq!(l.order(), [3, 2, 1]);
        assert_eq!(l.selected(0), Some(1));
    }

    #[test]
    fn last_left_tab_can_merge_into_right_strip() {
        let mut l = layout(&[1, 2]);
        l.drop_tab(2, DropTarget::Split);
        l.drop_tab(1, DropTarget::Strip { group: 1, index: 0 });
        assert!(!l.split());
        assert_eq!(l.tabs(0), [1, 2]);
        assert!(l.is_visible(1));
    }

    #[test]
    fn closing_only_tab_in_either_group_collapses() {
        for close_left in [false, true] {
            let mut l = layout(&[1, 2]);
            l.drop_tab(2, DropTarget::Split);
            let (closing, survivor) = if close_left { (1, 2) } else { (2, 1) };
            l.remove(closing);
            l.collapse_empty_group();
            assert!(!l.split());
            assert!(l.is_visible(survivor));
        }
    }

    #[test]
    fn removing_selection_selects_neighbour() {
        let mut l = layout(&[1, 2, 3]);
        l.select(2);
        l.remove(2);
        assert_eq!(l.selected(0), Some(3));
        l.remove(3);
        assert_eq!(l.selected(0), Some(1));
        l.remove(1);
        assert_eq!(l.selected(0), None);
    }

    #[test]
    fn restore_collapses_unusable_split() {
        let l = Layout::restore([(1, 1), (2, 1)], true);
        assert!(!l.split());
        assert_eq!(l.tabs(0), [1, 2]);
        let l = Layout::restore([(1, 0), (2, 1)], false);
        assert_eq!(l.tabs(0), [1, 2]);
        let l = Layout::restore([(1, 0), (2, 1)], true);
        assert!(l.split() && l.is_visible(2));
    }
}
