//! One space's arrangement: tab groups (panes) in a tree of side-by-side and
//! stacked splits. Pure data, so drag, split, merge and close rules are
//! testable without a window.
//!
//! The tree stays normalized: no empty pane, no split of one child, no split
//! directly inside a split of the same axis, and each split's shares sum to 1.

use crate::core::prefs::{Axis, Tree};

pub type TabId = u64;
/// Unique within one space's layout.
pub type PaneId = u64;
pub type SpaceId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn axis(self) -> Axis {
        match self {
            Edge::Left | Edge::Right => Axis::Row,
            Edge::Top | Edge::Bottom => Axis::Column,
        }
    }

    fn before(self) -> bool {
        matches!(self, Edge::Left | Edge::Top)
    }
}

/// Where a dragged tab would land inside a space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// Insert into a pane's tab strip before `index`.
    Strip { pane: PaneId, index: usize },
    /// Append to another pane.
    Pane { pane: PaneId },
    /// Split a pane, opening a new one with this tab at `edge`.
    Edge { pane: PaneId, edge: Edge },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: PaneId,
    pub tabs: Vec<TabId>,
    pub selected: Option<TabId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Pane(Group),
    Split { axis: Axis, children: Vec<(f32, Node)> },
}

impl Node {
    fn groups<'a>(&'a self, out: &mut Vec<&'a Group>) {
        match self {
            Node::Pane(group) => out.push(group),
            Node::Split { children, .. } => children.iter().for_each(|(_, c)| c.groups(out)),
        }
    }

    fn group_mut(&mut self, pane: PaneId) -> Option<&mut Group> {
        match self {
            Node::Pane(group) => (group.id == pane).then_some(group),
            Node::Split { children, .. } => children.iter_mut().find_map(|(_, c)| c.group_mut(pane)),
        }
    }

    /// Replaces pane `target` with a split of it and `new`; normalizing then
    /// folds that split into a parent of the same axis.
    fn insert(self, target: PaneId, new: &mut Option<Group>, edge: Edge) -> Node {
        match self {
            Node::Pane(group) if group.id == target => {
                let Some(new) = new.take() else { return Node::Pane(group) };
                let (first, second) = if edge.before() { (new, group) } else { (group, new) };
                Node::Split { axis: edge.axis(), children: vec![(0.5, Node::Pane(first)), (0.5, Node::Pane(second))] }
            }
            Node::Split { axis, children } => Node::Split {
                axis,
                children: children.into_iter().map(|(w, c)| (w, c.insert(target, new, edge))).collect(),
            },
            pane => pane,
        }
    }

    /// The smallest extent along `axis` that keeps every pane inside at
    /// least `pane`, with the shares as they are.
    pub fn min_extent(&self, axis: Axis, pane: f32) -> f32 {
        match self {
            Node::Pane(_) => pane,
            Node::Split { axis: own, children } if *own == axis => {
                children.iter().map(|(w, c)| c.min_extent(axis, pane) / w.max(f32::EPSILON)).fold(0., f32::max)
            }
            Node::Split { children, .. } => children.iter().map(|(_, c)| c.min_extent(axis, pane)).fold(0., f32::max),
        }
    }

    fn split_at_mut(&mut self, path: &[usize]) -> Option<(Axis, &mut Vec<(f32, Node)>)> {
        let Node::Split { axis, children } = self else { return None };
        match path.split_first() {
            None => Some((*axis, children)),
            Some((first, rest)) => children.get_mut(*first)?.1.split_at_mut(rest),
        }
    }
}

/// Drops empty panes (a removed child's share goes to its neighbour), lifts
/// single children and flattens same-axis nesting.
fn normalize(node: Node) -> Option<Node> {
    let Node::Split { axis, children } = node else {
        return match node {
            Node::Pane(group) if group.tabs.is_empty() => None,
            pane => Some(pane),
        };
    };
    let mut out: Vec<(f32, Node)> = Vec::new();
    let mut carry = 0.;
    for (weight, child) in children {
        // Bounded, so a hand-edited file cannot overflow the total or hide a pane.
        let weight = if weight.is_finite() && weight > 0. { weight.clamp(0.1, 10.) } else { 1. };
        match normalize(child) {
            None => match out.last_mut() {
                Some(previous) => previous.0 += weight,
                None => carry += weight,
            },
            Some(Node::Split { axis: inner, children }) if inner == axis => {
                out.extend(children.into_iter().map(|(w, c)| (w * (weight + carry), c)));
                carry = 0.;
            }
            Some(child) => {
                out.push((weight + carry, child));
                carry = 0.;
            }
        }
    }
    match out.len() {
        0 => None,
        1 => out.pop().map(|(_, child)| child),
        _ => {
            // Shares already summing to 1 stay exact, so a saved layout restores unchanged.
            let total: f32 = out.iter().map(|(w, _)| w).sum();
            if (total - 1.).abs() > 1e-5 {
                out.iter_mut().for_each(|(w, _)| *w /= total);
            }
            Some(Node::Split { axis, children: out })
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    root: Option<Node>,
    next_pane: PaneId,
}

/// What an absent space shows: nothing.
pub static EMPTY: Layout = Layout { root: None, next_pane: 0 };

impl Layout {
    pub fn root(&self) -> Option<&Node> {
        self.root.as_ref()
    }

    fn groups(&self) -> Vec<&Group> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            root.groups(&mut out);
        }
        out
    }

    /// Panes depth-first: left before right, top before bottom.
    pub fn panes(&self) -> Vec<PaneId> {
        self.groups().iter().map(|g| g.id).collect()
    }

    pub fn group(&self, pane: PaneId) -> Option<&Group> {
        self.groups().into_iter().find(|g| g.id == pane)
    }

    fn group_mut(&mut self, pane: PaneId) -> Option<&mut Group> {
        self.root.as_mut()?.group_mut(pane)
    }

    pub fn tabs(&self, pane: PaneId) -> &[TabId] {
        self.group(pane).map_or(&[], |g| g.tabs.as_slice())
    }

    pub fn selected(&self, pane: PaneId) -> Option<TabId> {
        self.group(pane)?.selected
    }

    pub fn pane_of(&self, tab: TabId) -> Option<PaneId> {
        self.groups().into_iter().find(|g| g.tabs.contains(&tab)).map(|g| g.id)
    }

    /// Every tab, panes depth-first: the persisted and Ctrl+Tab order.
    pub fn order(&self) -> Vec<TabId> {
        self.groups().into_iter().flat_map(|g| g.tabs.iter().copied()).collect()
    }

    pub fn len(&self) -> usize {
        self.groups().iter().map(|g| g.tabs.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn contains(&self, tab: TabId) -> bool {
        self.pane_of(tab).is_some()
    }

    pub fn is_visible(&self, tab: TabId) -> bool {
        self.groups().into_iter().any(|g| g.selected == Some(tab))
    }

    fn new_group(&mut self, tab: TabId) -> Group {
        self.next_pane += 1;
        Group { id: self.next_pane, tabs: vec![tab], selected: Some(tab) }
    }

    /// Adds a tab to `pane`, or the first pane when that is missing. Returns
    /// the pane it went to.
    pub fn add(&mut self, tab: TabId, pane: Option<PaneId>, index: Option<usize>) -> PaneId {
        let pane = pane.filter(|p| self.group(*p).is_some()).or_else(|| self.panes().first().copied());
        let Some(group) = pane.and_then(|p| self.group_mut(p)) else {
            let group = self.new_group(tab);
            let id = group.id;
            self.root = Some(Node::Pane(group));
            return id;
        };
        let at = index.unwrap_or(group.tabs.len()).min(group.tabs.len());
        group.tabs.insert(at, tab);
        group.selected.get_or_insert(tab);
        group.id
    }

    pub fn select(&mut self, tab: TabId) {
        if let Some(group) = self.pane_of(tab).and_then(|p| self.group_mut(p)) {
            group.selected = Some(tab);
        }
    }

    /// Takes a tab out of its pane, leaving the pane in place even if empty.
    /// A removed selection passes to its neighbour.
    fn take(&mut self, tab: TabId) -> Option<PaneId> {
        let pane = self.pane_of(tab)?;
        let group = self.group_mut(pane)?;
        let index = group.tabs.iter().position(|t| *t == tab)?;
        group.tabs.remove(index);
        if group.selected == Some(tab) {
            group.selected = group.tabs.get(index.min(group.tabs.len().saturating_sub(1))).copied();
        }
        Some(pane)
    }

    fn prune(&mut self) {
        self.root = self.root.take().and_then(normalize);
    }

    /// Removes a tab and returns the pane that should take focus: its own if
    /// it still has tabs, otherwise the nearest remaining one, preferring the
    /// pane before it.
    pub fn remove(&mut self, tab: TabId) -> Option<PaneId> {
        let before = self.panes();
        let pane = self.take(tab)?;
        self.prune();
        let after = self.panes();
        let index = before.iter().position(|p| *p == pane)?;
        before[..=index].iter().rev().chain(&before[index + 1..]).copied().find(|p| after.contains(p))
    }

    /// Whether `target` is a meaningful destination for `tab`.
    pub fn accepts(&self, tab: TabId, target: DropTarget) -> bool {
        let Some(from) = self.pane_of(tab) else { return false };
        match target {
            DropTarget::Strip { pane, .. } => self.group(pane).is_some(),
            DropTarget::Pane { pane } => pane != from && self.group(pane).is_some(),
            // Splitting a pane off its only tab would put it back where it was.
            DropTarget::Edge { pane, .. } => {
                self.group(pane).is_some() && !(pane == from && self.tabs(from).len() == 1)
            }
        }
    }

    /// Applies a drop and selects the tab. Returns whether it applied.
    pub fn drop_tab(&mut self, tab: TabId, target: DropTarget) -> bool {
        if !self.accepts(tab, target) {
            return false;
        }
        let from = self.pane_of(tab);
        match target {
            DropTarget::Strip { pane, index } => {
                let mut index = index;
                if from == Some(pane) {
                    let position = self.tabs(pane).iter().position(|t| *t == tab).unwrap();
                    if position < index {
                        index -= 1;
                    }
                }
                self.take(tab);
                self.add(tab, Some(pane), Some(index));
            }
            DropTarget::Pane { pane } => {
                self.take(tab);
                self.add(tab, Some(pane), None);
            }
            DropTarget::Edge { pane, edge } => {
                // A pane left empty goes first, so its share returns to its
                // neighbour rather than to the new pane.
                self.take(tab);
                self.prune();
                let mut new = Some(self.new_group(tab));
                self.root = self.root.take().map(|root| root.insert(pane, &mut new, edge));
            }
        }
        self.prune();
        self.select(tab);
        true
    }

    /// Moves the boundary after child `index` of the split at `path` to
    /// `at`, a fraction of that split's extent. No pane on either side, even
    /// inside a nested split, drops below `min`, unless the pair is too small
    /// for both.
    pub fn resize(&mut self, path: &[usize], index: usize, at: f32, min: f32) {
        let Some((axis, children)) = self.root.as_mut().and_then(|r| r.split_at_mut(path)) else { return };
        if index + 1 >= children.len() {
            return;
        }
        let start: f32 = children[..index].iter().map(|(w, _)| w).sum();
        let pair = children[index].0 + children[index + 1].0;
        let (mut low, mut high) =
            (children[index].1.min_extent(axis, min), children[index + 1].1.min_extent(axis, min));
        if low + high > pair {
            (low, high) = (pair * low / (low + high), pair * high / (low + high));
        }
        let first = (at - start).clamp(low, (pair - high).max(low));
        children[index].0 = first;
        children[index + 1].0 = pair - first;
    }

    /// Gives every child of the split at `path` the same share.
    pub fn equalize(&mut self, path: &[usize]) {
        if let Some((_, children)) = self.root.as_mut().and_then(|r| r.split_at_mut(path)) {
            let share = 1. / children.len() as f32;
            children.iter_mut().for_each(|(w, _)| *w = share);
        }
    }

    pub fn restore(tree: Tree<TabId>) -> Self {
        fn build(tree: Tree<TabId>, layout: &mut Layout) -> Node {
            match tree {
                Tree::Pane { tabs, selected } => {
                    layout.next_pane += 1;
                    let selected = tabs.get(selected).or(tabs.first()).copied();
                    Node::Pane(Group { id: layout.next_pane, tabs, selected })
                }
                Tree::Split { axis, children } => {
                    Node::Split { axis, children: children.into_iter().map(|(w, c)| (w, build(c, layout))).collect() }
                }
            }
        }
        let mut layout = Layout::default();
        layout.root = Some(build(tree, &mut layout));
        layout.prune();
        layout
    }

    pub fn to_tree(&self) -> Option<Tree<TabId>> {
        fn tree(node: &Node) -> Tree<TabId> {
            match node {
                Node::Pane(group) => Tree::Pane {
                    tabs: group.tabs.clone(),
                    selected: group.selected.and_then(|s| group.tabs.iter().position(|t| *t == s)).unwrap_or(0),
                },
                Node::Split { axis, children } => {
                    Tree::Split { axis: *axis, children: children.iter().map(|(w, c)| (*w, tree(c))).collect() }
                }
            }
        }
        self.root.as_ref().map(tree)
    }
}

/// One Herdr space and the conversations with its terminals, in their own
/// arrangement; shown one at a time beside the space list.
#[derive(Clone, Debug)]
pub struct Space {
    pub id: SpaceId,
    /// Herdr's id for it. Empty for conversations whose terminal is outside
    /// Herdr's window.
    pub key: String,
    pub name: String,
    /// `None` is this computer.
    pub machine: Option<String>,
    /// Still in Herdr's window (for the empty key: such terminals exist). A
    /// closed space stays listed while it has conversations.
    pub live: bool,
    pub layout: Layout,
    /// The tab focused when this space was last on screen.
    pub active: Option<TabId>,
}

/// The space-list group of a space: its machine, or "other" for the empty key.
pub fn group_of(key: &str, machine: Option<&str>) -> String {
    match (machine, key.is_empty()) {
        (_, true) => "other".into(),
        (None, false) => "local".into(),
        (Some(machine), false) => format!("machine:{machine}"),
    }
}

impl Space {
    pub fn group(&self) -> String {
        group_of(&self.key, self.machine.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(tabs: &[TabId]) -> Layout {
        Layout::restore(Tree::pane(tabs.to_vec()))
    }

    fn shape(l: &Layout) -> String {
        fn walk(node: &Node) -> String {
            match node {
                Node::Pane(g) => format!("{:?}", g.tabs),
                Node::Split { axis, children } => {
                    let inner: Vec<String> = children.iter().map(|(_, c)| walk(c)).collect();
                    format!("{}({})", if *axis == Axis::Row { "R" } else { "C" }, inner.join(" "))
                }
            }
        }
        l.root().map_or("-".into(), walk)
    }

    fn weights(l: &Layout, path: &[usize]) -> Vec<f32> {
        let mut node = l.root().unwrap();
        for i in path {
            let Node::Split { children, .. } = node else { panic!() };
            node = &children[*i].1;
        }
        let Node::Split { children, .. } = node else { panic!() };
        children.iter().map(|(w, _)| (w * 1000.).round() / 1000.).collect()
    }

    fn pane(l: &Layout, tab: TabId) -> PaneId {
        l.pane_of(tab).unwrap()
    }

    #[test]
    fn split_right_then_merge_back_collapses() {
        let mut l = layout(&[1, 2]);
        let p = pane(&l, 1);
        assert!(l.drop_tab(1, DropTarget::Edge { pane: p, edge: Edge::Right }));
        assert_eq!(shape(&l), "R([2] [1])");
        assert!(l.is_visible(1) && l.is_visible(2));
        assert!(l.drop_tab(1, DropTarget::Strip { pane: pane(&l, 2), index: 0 }));
        assert_eq!(shape(&l), "[1, 2]");
        assert_eq!(l.panes().len(), 1);
    }

    #[test]
    fn edges_split_in_both_directions_into_a_grid() {
        let mut l = layout(&[1, 2, 3, 4]);
        let first = pane(&l, 1);
        l.drop_tab(2, DropTarget::Edge { pane: first, edge: Edge::Right });
        l.drop_tab(3, DropTarget::Edge { pane: first, edge: Edge::Bottom });
        l.drop_tab(4, DropTarget::Edge { pane: pane(&l, 2), edge: Edge::Bottom });
        assert_eq!(shape(&l), "R(C([1] [3]) C([2] [4]))");
        // A same-axis split folds into its parent and halves the target's share.
        l.drop_tab(1, DropTarget::Edge { pane: pane(&l, 3), edge: Edge::Left });
        assert_eq!(shape(&l), "R([1] [3] C([2] [4]))");
        assert_eq!(weights(&l, &[]), [0.25, 0.25, 0.5]);
        assert_eq!(l.order(), [1, 3, 2, 4]);
    }

    #[test]
    fn only_tab_cannot_split_its_own_pane_and_own_pane_is_not_a_target() {
        let mut l = layout(&[1]);
        let p = pane(&l, 1);
        assert!(!l.drop_tab(1, DropTarget::Edge { pane: p, edge: Edge::Right }));
        assert!(!l.accepts(1, DropTarget::Pane { pane: p }));
        let mut l = layout(&[1, 2, 3]);
        l.drop_tab(3, DropTarget::Edge { pane: pane(&l, 1), edge: Edge::Right });
        let right = pane(&l, 3);
        assert!(!l.accepts(3, DropTarget::Pane { pane: right }));
        assert!(l.drop_tab(1, DropTarget::Pane { pane: right }));
        assert_eq!(shape(&l), "R([2] [3, 1])");
        // A lone tab may still move beside another pane.
        let left = pane(&l, 2);
        assert!(l.drop_tab(2, DropTarget::Edge { pane: right, edge: Edge::Bottom }));
        assert_eq!(shape(&l), "C([3, 1] [2])");
        assert!(l.group(left).is_none());
    }

    #[test]
    fn reorder_within_strip() {
        let mut l = layout(&[1, 2, 3]);
        let p = pane(&l, 1);
        l.drop_tab(3, DropTarget::Strip { pane: p, index: 0 });
        assert_eq!(l.order(), [3, 1, 2]);
        l.drop_tab(3, DropTarget::Strip { pane: p, index: 2 });
        assert_eq!(l.order(), [1, 3, 2]);
        l.drop_tab(1, DropTarget::Strip { pane: p, index: 3 });
        assert_eq!(l.order(), [3, 2, 1]);
        assert_eq!(l.selected(p), Some(1));
    }

    #[test]
    fn closing_the_last_tab_of_a_pane_gives_its_share_to_a_neighbour() {
        let mut l = layout(&[1, 2, 3]);
        let p = pane(&l, 1);
        l.drop_tab(2, DropTarget::Edge { pane: p, edge: Edge::Right });
        l.drop_tab(3, DropTarget::Edge { pane: pane(&l, 2), edge: Edge::Right });
        l.resize(&[], 0, 0.5, 0.1);
        assert_eq!(weights(&l, &[]), [0.5, 0.25, 0.25]);
        let middle = pane(&l, 2);
        assert_eq!(l.remove(2), Some(p));
        assert_eq!(shape(&l), "R([1] [3])");
        assert_eq!(weights(&l, &[]), [0.75, 0.25]);
        assert!(l.group(middle).is_none());
        // The first pane has no pane before it, so the next one takes focus.
        let right = pane(&l, 3);
        assert_eq!(l.remove(1), Some(right));
        assert_eq!(shape(&l), "[3]");
        assert_eq!(l.remove(3), None);
        assert!(l.is_empty() && l.order().is_empty());
    }

    #[test]
    fn removing_selection_selects_neighbour() {
        let mut l = layout(&[1, 2, 3]);
        let p = pane(&l, 1);
        l.select(2);
        assert_eq!(l.remove(2), Some(p));
        assert_eq!(l.selected(p), Some(3));
        l.remove(3);
        assert_eq!(l.selected(p), Some(1));
    }

    #[test]
    fn resize_keeps_a_minimum_and_equalize_resets() {
        let mut l = layout(&[1, 2]);
        l.drop_tab(2, DropTarget::Edge { pane: pane(&l, 1), edge: Edge::Bottom });
        l.resize(&[], 0, 0.95, 0.2);
        assert_eq!(weights(&l, &[]), [0.8, 0.2]);
        l.resize(&[], 0, 0.3, 0.2);
        assert_eq!(weights(&l, &[]), [0.3, 0.7]);
        l.resize(&[], 0, 0.3, 0.9);
        assert_eq!(weights(&l, &[]), [0.5, 0.5]);
        l.resize(&[], 1, 0.3, 0.2);
        l.resize(&[4], 0, 0.3, 0.2);
        assert_eq!(weights(&l, &[]), [0.5, 0.5]);
        l.resize(&[], 0, 0.1, 0.2);
        l.equalize(&[]);
        assert_eq!(weights(&l, &[]), [0.5, 0.5]);
    }

    #[test]
    fn a_lone_tab_moved_beside_its_neighbour_keeps_even_shares() {
        let mut l = layout(&[1, 2]);
        l.drop_tab(2, DropTarget::Edge { pane: pane(&l, 1), edge: Edge::Right });
        // Back where it was, and swapped: still half each.
        assert!(l.drop_tab(2, DropTarget::Edge { pane: pane(&l, 1), edge: Edge::Right }));
        assert_eq!((shape(&l), weights(&l, &[])), ("R([1] [2])".into(), vec![0.5, 0.5]));
        assert!(l.drop_tab(2, DropTarget::Edge { pane: pane(&l, 1), edge: Edge::Left }));
        assert_eq!((shape(&l), weights(&l, &[])), ("R([2] [1])".into(), vec![0.5, 0.5]));
    }

    #[test]
    fn outer_dividers_respect_nested_panes() {
        // R([1] C(R([2] [3]) [4])): the column holds two panes side by side.
        let mut l = layout(&[1, 2, 3, 4]);
        let first = pane(&l, 1);
        l.drop_tab(2, DropTarget::Edge { pane: first, edge: Edge::Right });
        l.drop_tab(4, DropTarget::Edge { pane: pane(&l, 2), edge: Edge::Bottom });
        l.drop_tab(3, DropTarget::Edge { pane: pane(&l, 2), edge: Edge::Right });
        assert_eq!(shape(&l), "R([1] C(R([2] [3]) [4]))");
        l.resize(&[], 0, 0.9, 0.2);
        assert_eq!(weights(&l, &[]), [0.6, 0.4]);
        // Too small for both: the minimums share what there is.
        l.resize(&[], 0, 0.9, 0.5);
        assert_eq!(weights(&l, &[]), [0.333, 0.667]);
    }

    #[test]
    fn extreme_saved_shares_stay_bounded() {
        let tree = Tree::Split {
            axis: Axis::Row,
            children: vec![(3e38, Tree::pane(vec![1])), (3e38, Tree::pane(vec![2])), (1e-30, Tree::pane(vec![3]))],
        };
        let l = Layout::restore(tree);
        let Some(Node::Split { children, .. }) = l.root() else { panic!() };
        let w: Vec<f32> = children.iter().map(|(w, _)| *w).collect();
        assert!(w.iter().all(|w| *w >= 0.004) && (w.iter().sum::<f32>() - 1.).abs() < 1e-4, "{w:?}");
    }

    #[test]
    fn restore_normalizes_and_round_trips() {
        let tree = Tree::Split {
            axis: Axis::Row,
            children: vec![
                (2., Tree::Pane { tabs: vec![1, 2], selected: 1 }),
                (
                    1.,
                    Tree::Split {
                        axis: Axis::Row,
                        children: vec![(1., Tree::pane(vec![3])), (1., Tree::pane(vec![4]))],
                    },
                ),
            ],
        };
        let l = Layout::restore(tree);
        assert_eq!(shape(&l), "R([1, 2] [3] [4])");
        assert_eq!(weights(&l, &[]), [0.667, 0.167, 0.167]);
        assert_eq!(l.selected(pane(&l, 1)), Some(2));
        let again = Layout::restore(l.to_tree().unwrap());
        assert_eq!(again.root(), l.root());
    }
}
