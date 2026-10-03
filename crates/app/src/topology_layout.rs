//! The Topology layout (W11): kind columns, a config row under each band, barycenter ordering, and
//! bands packed into band-columns, by hand (decision 20). Pure and in graph units (1 unit is 1 px
//! at zoom 1). A layout made with the previous one keeps the order, the vertical offsets, and the
//! band-column of what was there, so a new pod appears in place (W11 pin 3).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use gpui_kit::SharedString;

use crate::topology_graph::{GroupBy, NodeId, Relation, TopologyGraph, TopologyKind, TopologyNode};
use crate::topology_route::{EdgeRoute, route_edges};

/// A card is at least this wide, and as wide as its longest name needs up to `MAX_NODE_WIDTH`.
pub(crate) const MIN_NODE_WIDTH: f32 = 200.;
const MAX_NODE_WIDTH: f32 = 280.;
pub(crate) const NODE_HEIGHT: f32 = 60.;
/// The gap between two columns of cards, where the edges run.
const GUTTER: f32 = 50.;
/// What a card holds besides its name: the accent bar, the chip, and the padding.
const CARD_CHROME: f32 = 63.;
/// The width of one character of a name (13 px in the UI font), which sets the card width.
const NAME_CHAR_WIDTH: f32 = 7.2;
/// The column pitch of a graph whose names are short.
#[cfg(test)]
pub(crate) const COLUMN_PITCH: f32 = MIN_NODE_WIDTH + GUTTER;
pub(crate) const ROW_PITCH: f32 = 80.;
const CONFIG_GAP: f32 = 56.;
const BAND_GAP: f32 = 44.;
const BAND_PAD: f32 = 16.;
/// The strip of a band that holds its title pill.
const BAND_TITLE: f32 = 32.;
const MARGIN: f32 = 24.;
/// Barycenter passes over the columns, alternating down and up.
const SWEEPS: usize = 4;
/// The kind columns: Ingress and HPA, then Service and the workloads, ReplicaSets, and pods. A
/// column no node of the graph uses is dropped before the bands are laid out.
const COLUMNS: usize = 4;
/// A graph is never spread over more band-columns than this.
const MAX_BAND_COLUMNS: usize = 8;
const UNGROUPED: &str = "Ungrouped";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GraphPoint {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GraphRect {
    pub(crate) origin: GraphPoint,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl GraphRect {
    pub(crate) fn right(&self) -> f32 {
        self.origin.x + self.width
    }

    pub(crate) fn bottom(&self) -> f32 {
        self.origin.y + self.height
    }

    pub(crate) fn center(&self) -> GraphPoint {
        GraphPoint {
            x: self.origin.x + self.width / 2.,
            y: self.origin.y + self.height / 2.,
        }
    }

    fn node(x: f32, y: f32, width: f32) -> Self {
        Self {
            origin: GraphPoint { x, y },
            width,
            height: NODE_HEIGHT,
        }
    }

    fn spanning(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            origin: GraphPoint { x: left, y: top },
            width: right - left,
            height: bottom - top,
        }
    }
}

/// Where a kind sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    Column(u8),
    /// Under the band, below its columns.
    ConfigRow,
    /// Under the config row (RBAC layer): accounts, bindings, and roles read left to right.
    AccessRow,
}

/// A column of the band, or its config row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Column(u8),
    ConfigRow,
    AccessRow,
}

pub(crate) struct Band {
    /// The app name, or `Ungrouped`; `None` for a component band, which is not drawn.
    pub(crate) title: Option<SharedString>,
    pub(crate) rect: GraphRect,
}

/// The nodes of one slot of one band in top-to-bottom order: the seed of the next layout.
struct SlotOrder {
    ids: Vec<NodeId>,
    /// How far the column starts below the top of its band (centering); 0 for the config row.
    offset: f32,
}

/// The measures of one layout: how wide a card is and which kind columns the graph uses.
struct Metrics {
    node_width: f32,
    /// The kind columns (`Placement::Column`) some node uses, in order: a column's place here is
    /// its place on the canvas.
    used_columns: Vec<u8>,
}

impl Metrics {
    fn of(graph: &TopologyGraph) -> Self {
        let longest = graph
            .nodes
            .iter()
            .map(|node| node.name.chars().count())
            .max()
            .unwrap_or(0);
        let mut used_columns: Vec<u8> = graph
            .nodes
            .iter()
            .filter_map(|node| match slot_of(node) {
                Slot::Column(column) => Some(column),
                Slot::ConfigRow | Slot::AccessRow => None,
            })
            .collect();
        used_columns.sort_unstable();
        used_columns.dedup();
        Self {
            node_width: (CARD_CHROME + longest as f32 * NAME_CHAR_WIDTH)
                .clamp(MIN_NODE_WIDTH, MAX_NODE_WIDTH)
                .ceil(),
            used_columns,
        }
    }

    /// How many columns a band has; a graph of config nodes alone still has one slot.
    fn column_count(&self) -> usize {
        self.used_columns.len().max(1)
    }

    /// The place of a kind column among the used ones.
    fn place_of(&self, column: u8) -> usize {
        self.used_columns
            .iter()
            .position(|used| *used == column)
            .unwrap_or(0)
    }

    fn pitch(&self) -> f32 {
        self.node_width + GUTTER
    }

    /// The width of every band: its columns and its padding, so bands line up in a band-column.
    fn band_width(&self) -> f32 {
        2. * BAND_PAD + (self.column_count() - 1) as f32 * self.pitch() + self.node_width
    }

    fn column_x(&self, pad: f32, place: usize) -> f32 {
        pad + place as f32 * self.pitch()
    }
}

pub(crate) struct TopologyLayout {
    /// One per node of the graph, in node order.
    pub(crate) rects: Vec<GraphRect>,
    /// One per edge of the graph, in edge order: the way it runs between its cards.
    pub(crate) routes: Vec<EdgeRoute>,
    pub(crate) bands: Vec<Band>,
    /// Everything, plus the margin.
    pub(crate) extent: GraphRect,
    order: Vec<SlotOrder>,
    /// The band-column of each node, and how many band-columns there are: what the next layout
    /// keeps.
    node_columns: HashMap<NodeId, usize>,
    band_columns: usize,
}

/// The nodes and edges of a graph by identity. Two graphs of the same structure keep their layout:
/// only tones and captions changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GraphStructure {
    pub(crate) nodes: Vec<NodeId>,
    pub(crate) edges: Vec<(NodeId, NodeId, Relation)>,
}

pub(crate) fn structure(graph: &TopologyGraph) -> GraphStructure {
    GraphStructure {
        nodes: graph.nodes.iter().map(|node| node.id.clone()).collect(),
        edges: graph
            .edges
            .iter()
            .map(|edge| {
                (
                    graph.nodes[edge.from].id.clone(),
                    graph.nodes[edge.to].id.clone(),
                    edge.relation,
                )
            })
            .collect(),
    }
}

/// What the previous layout says: each node's rank in its slot, the offset of its column, and the
/// band-column it was in.
struct Seed<'a> {
    rank: HashMap<NodeId, usize>,
    offset: HashMap<NodeId, f32>,
    band_column: &'a HashMap<NodeId, usize>,
    band_columns: usize,
}

impl<'a> Seed<'a> {
    fn of(previous: &'a TopologyLayout) -> Self {
        let mut rank = HashMap::new();
        let mut offset = HashMap::new();
        for slot in &previous.order {
            for (position, id) in slot.ids.iter().enumerate() {
                rank.insert(id.clone(), position);
                offset.insert(id.clone(), slot.offset);
            }
        }
        Self {
            rank,
            offset,
            band_column: &previous.node_columns,
            band_columns: previous.band_columns.max(1),
        }
    }
}

/// The nodes of one band, before they are ordered.
struct BandPlan {
    title: Option<SharedString>,
    nodes: Vec<usize>,
}

/// Lays `graph` out. `aspect` is the width over the height of the canvas the graph is shown in:
/// from scratch, the bands flow into as many band-columns as make the extent look like it.
/// `pins` move nodes to the origins the user dragged them to. `previous` seeds the order, the
/// offsets of the columns, and the band-columns from an earlier layout of the same namespace and
/// grouping, and skips the sweeps; without it the order comes from sorting and the sweeps.
pub(crate) fn layout(
    graph: &TopologyGraph,
    group_by: GroupBy,
    aspect: f32,
    pins: &HashMap<NodeId, GraphPoint>,
    previous: Option<&TopologyLayout>,
) -> TopologyLayout {
    let seed = previous.map(Seed::of);
    let neighbours = neighbours(graph);
    let metrics = Metrics::of(graph);
    let mut rects = vec![GraphRect::node(0., 0., metrics.node_width); graph.nodes.len()];
    let plans = plan_bands(graph, group_by);
    // Every band is placed at the origin first: its height decides the band-column it goes to.
    let mut placed: Vec<PlacedBand> = plans
        .iter()
        .map(|plan| {
            place_band(
                graph,
                plan,
                &neighbours,
                seed.as_ref(),
                &metrics,
                &mut rects,
            )
        })
        .collect();
    let heights: Vec<f32> = placed.iter().map(|band| band.rect.height).collect();
    let remembered: Vec<Option<usize>> = plans
        .iter()
        .map(|plan| {
            seed.as_ref()
                .and_then(|seed| remembered_column(graph, plan, seed))
        })
        .collect();
    let columns = match &seed {
        Some(seed) => assign_columns(&heights, &remembered, seed.band_columns),
        None => best_columns(&heights, aspect, metrics.band_width()),
    };
    let band_columns = columns.iter().max().map_or(1, |last| last + 1);
    let mut bottoms = vec![MARGIN; band_columns];
    let mut bands = Vec::new();
    let mut order = Vec::new();
    let mut node_columns = HashMap::new();
    for ((plan, band), column) in plans.into_iter().zip(&mut placed).zip(columns) {
        let dx = MARGIN + column as f32 * (metrics.band_width() + BAND_GAP);
        let dy = bottoms[column];
        bottoms[column] = dy + band.rect.height + BAND_GAP;
        for &index in &plan.nodes {
            rects[index].origin.x += dx;
            rects[index].origin.y += dy;
            node_columns.insert(graph.nodes[index].id.clone(), column);
        }
        band.rect.origin = GraphPoint { x: dx, y: dy };
        order.append(&mut band.order);
        bands.push(Band {
            title: plan.title,
            rect: band.rect,
        });
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        if let Some(pin) = pins.get(&node.id) {
            rects[index].origin = *pin;
        }
    }
    let extent = extent_of(&rects, &bands);
    let frames: Vec<GraphRect> = bands.iter().map(|band| band.rect).collect();
    let routes = route_edges(graph, &rects, &frames);
    TopologyLayout {
        rects,
        routes,
        bands,
        extent,
        order,
        node_columns,
        band_columns,
    }
}

/// The band-column a band was in, by its first node that the previous layout knew.
fn remembered_column(graph: &TopologyGraph, plan: &BandPlan, seed: &Seed) -> Option<usize> {
    plan.nodes
        .iter()
        .find_map(|&index| seed.band_column.get(&graph.nodes[index].id))
        .copied()
}

/// Flows bands, in order, into `count` band-columns: a band with a remembered column goes there, the
/// others into the shortest column so far (the first one on a tie).
fn assign_columns(heights: &[f32], remembered: &[Option<usize>], count: usize) -> Vec<usize> {
    let mut bottoms = vec![0.; count.max(1)];
    heights
        .iter()
        .zip(remembered)
        .map(|(height, remembered)| {
            let column = remembered
                .filter(|column| *column < bottoms.len())
                .unwrap_or_else(|| shortest(&bottoms));
            bottoms[column] += height + BAND_GAP;
            column
        })
        .collect()
}

fn shortest(bottoms: &[f32]) -> usize {
    bottoms
        .iter()
        .enumerate()
        .fold(0, |best, (column, bottom)| {
            if *bottom < bottoms[best] {
                column
            } else {
                best
            }
        })
}

/// The flow whose extent has the aspect closest to `aspect`, trying one band-column up to
/// `MAX_BAND_COLUMNS` (the fewer columns on a tie).
fn best_columns(heights: &[f32], aspect: f32, band_width: f32) -> Vec<usize> {
    let remembered = vec![None; heights.len()];
    let mut best: Option<(f32, Vec<usize>)> = None;
    for count in 1..=heights.len().clamp(1, MAX_BAND_COLUMNS) {
        let columns = assign_columns(heights, &remembered, count);
        let used = columns.iter().max().map_or(1, |last| last + 1);
        let mut bottoms = vec![0.; used];
        for (height, column) in heights.iter().zip(&columns) {
            bottoms[*column] += height + BAND_GAP;
        }
        let tallest = bottoms.iter().copied().fold(0., f32::max) - BAND_GAP;
        let width = used as f32 * band_width + (used - 1) as f32 * BAND_GAP;
        let miss = ((width / tallest.max(1.)) / aspect.max(0.01)).ln().abs();
        if best.as_ref().is_none_or(|(known, _)| miss < *known) {
            best = Some((miss, columns));
        }
    }
    best.map_or_else(Vec::new, |(_, columns)| columns)
}

fn extent_of(rects: &[GraphRect], bands: &[Band]) -> GraphRect {
    let all = || rects.iter().chain(bands.iter().map(|band| &band.rect));
    let left = all()
        .map(|rect| rect.origin.x)
        .fold(f32::INFINITY, f32::min);
    let top = all()
        .map(|rect| rect.origin.y)
        .fold(f32::INFINITY, f32::min);
    let right = all()
        .map(GraphRect::right)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = all()
        .map(GraphRect::bottom)
        .fold(f32::NEG_INFINITY, f32::max);
    if !left.is_finite() {
        return GraphRect::spanning(0., 0., 2. * MARGIN, 2. * MARGIN);
    }
    GraphRect::spanning(left - MARGIN, top - MARGIN, right + MARGIN, bottom + MARGIN)
}

/// The undirected neighbours of each node.
fn neighbours(graph: &TopologyGraph) -> Vec<Vec<usize>> {
    let mut neighbours = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        neighbours[edge.from].push(edge.to);
        neighbours[edge.to].push(edge.from);
    }
    neighbours
}

fn slot_of(node: &TopologyNode) -> Slot {
    match node.kind.placement() {
        Placement::Column(column) => Slot::Column(column.min(COLUMNS as u8 - 1)),
        Placement::ConfigRow => Slot::ConfigRow,
        Placement::AccessRow => Slot::AccessRow,
    }
}

/// Step 1: component bands, largest first with the single nodes last, or app bands by name with
/// `Ungrouped` last.
fn plan_bands(graph: &TopologyGraph, group_by: GroupBy) -> Vec<BandPlan> {
    match group_by {
        GroupBy::Components => component_bands(graph),
        GroupBy::App => app_bands(graph),
    }
}

fn app_bands(graph: &TopologyGraph) -> Vec<BandPlan> {
    let mut by_app: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut ungrouped = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        match &node.group {
            Some(group) => by_app.entry(group).or_default().push(index),
            None => ungrouped.push(index),
        }
    }
    let mut plans: Vec<BandPlan> = by_app
        .into_iter()
        .map(|(app, nodes)| BandPlan {
            title: Some(app.to_owned().into()),
            nodes,
        })
        .collect();
    if !ungrouped.is_empty() {
        plans.push(BandPlan {
            title: Some(UNGROUPED.into()),
            nodes: ungrouped,
        });
    }
    plans
}

fn component_bands(graph: &TopologyGraph) -> Vec<BandPlan> {
    let mut parent: Vec<usize> = (0..graph.nodes.len()).collect();
    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    for edge in &graph.edges {
        let (from, to) = (root(&mut parent, edge.from), root(&mut parent, edge.to));
        // The smaller index stays the root, so a component is named by its smallest node.
        if from < to {
            parent[to] = from;
        } else {
            parent[from] = to;
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for node in 0..graph.nodes.len() {
        let component = root(&mut parent, node);
        components.entry(component).or_default().push(node);
    }
    let (mut groups, singles): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        components.into_values().partition(|nodes| nodes.len() >= 2);
    // Nodes are in `NodeId` order, so the first one of each is its smallest id.
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    let mut plans: Vec<BandPlan> = groups
        .into_iter()
        .map(|nodes| BandPlan { title: None, nodes })
        .collect();
    let singles: Vec<usize> = singles.into_iter().flatten().collect();
    if !singles.is_empty() {
        plans.push(BandPlan {
            title: None,
            nodes: singles,
        });
    }
    plans
}

struct PlacedBand {
    rect: GraphRect,
    order: Vec<SlotOrder>,
}

/// Steps 2 to 6 for one band, with its top-left corner at the origin: order, stack, center, config
/// row. The caller moves it into its band-column.
fn place_band(
    graph: &TopologyGraph,
    plan: &BandPlan,
    neighbours: &[Vec<usize>],
    seed: Option<&Seed>,
    metrics: &Metrics,
    rects: &mut [GraphRect],
) -> PlacedBand {
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); metrics.column_count()];
    let mut config = Vec::new();
    let mut access = Vec::new();
    for &index in &plan.nodes {
        match slot_of(&graph.nodes[index]) {
            Slot::Column(column) => columns[metrics.place_of(column)].push(index),
            Slot::ConfigRow => config.push(index),
            Slot::AccessRow => access.push(index),
        }
    }
    for column in &mut columns {
        sort_slot(graph, column, seed);
    }
    if seed.is_none() {
        sweep_columns(graph, &mut columns, neighbours);
    }
    let (pad, title) = match plan.title {
        Some(_) => (BAND_PAD, BAND_TITLE),
        None => (0., 0.),
    };
    let body_top = pad + title;
    let tallest = columns.iter().map(Vec::len).max().unwrap_or(0);
    let mut order = Vec::new();
    let mut columns_bottom = body_top;
    for (column, nodes) in columns.iter().enumerate() {
        if nodes.is_empty() {
            continue;
        }
        let offset = column_offset(graph, nodes, seed, tallest);
        let x = metrics.column_x(pad, column);
        for (row, &index) in nodes.iter().enumerate() {
            let y = body_top + offset + row as f32 * ROW_PITCH;
            rects[index] = GraphRect::node(x, y, metrics.node_width);
            columns_bottom = columns_bottom.max(rects[index].bottom());
        }
        order.push(SlotOrder {
            ids: ids_of(graph, nodes),
            offset,
        });
    }
    let mut bottom = columns_bottom;
    if !config.is_empty() {
        let config_top = if tallest == 0 {
            body_top
        } else {
            columns_bottom + CONFIG_GAP
        };
        let placed = place_config_row(graph, &config, seed, metrics, pad, config_top, rects);
        bottom = bottom.max(config_top + (placed.rows - 1) as f32 * ROW_PITCH + NODE_HEIGHT);
        order.push(placed.order);
    }
    if !access.is_empty() {
        let has_rows_above = tallest > 0 || !config.is_empty();
        let access_top = if has_rows_above {
            bottom + CONFIG_GAP
        } else {
            body_top
        };
        let placed = place_access_row(graph, &access, seed, metrics, pad, access_top, rects);
        bottom = bottom.max(access_top + (placed.rows - 1) as f32 * ROW_PITCH + NODE_HEIGHT);
        order.push(placed.order);
    }
    PlacedBand {
        rect: GraphRect::spanning(0., 0., metrics.band_width(), bottom + pad),
        order,
    }
}

fn ids_of(graph: &TopologyGraph, nodes: &[usize]) -> Vec<NodeId> {
    nodes
        .iter()
        .map(|&index| graph.nodes[index].id.clone())
        .collect()
}

/// Step 2: the order of one slot. Without a seed: by `(kind, NodeId)`. With one: what was there
/// keeps its place, and what is new follows in `(kind, NodeId)` order.
fn sort_slot(graph: &TopologyGraph, nodes: &mut [usize], seed: Option<&Seed>) {
    nodes.sort_by(|&a, &b| {
        let rank = |index: usize| {
            seed.and_then(|seed| seed.rank.get(&graph.nodes[index].id))
                .copied()
                .unwrap_or(usize::MAX)
        };
        let key = |index: usize| (graph.nodes[index].kind, &graph.nodes[index].id);
        rank(a).cmp(&rank(b)).then_with(|| key(a).cmp(&key(b)))
    });
}

/// Step 4: a shorter column is centered, unless the previous layout had this column at some
/// offset: then it stays there, so the nodes already drawn do not move.
fn column_offset(
    graph: &TopologyGraph,
    nodes: &[usize],
    seed: Option<&Seed>,
    tallest: usize,
) -> f32 {
    let remembered = seed.and_then(|seed| {
        nodes
            .iter()
            .find_map(|&index| seed.offset.get(&graph.nodes[index].id))
            .copied()
    });
    remembered.unwrap_or((tallest - nodes.len()) as f32 * ROW_PITCH / 2.)
}

/// Step 3: barycenter sweeps, alternating down (columns 1 to 3, by the neighbours on the left) and
/// up (columns 2 to 0, by the neighbours on the right).
fn sweep_columns(graph: &TopologyGraph, columns: &mut [Vec<usize>], neighbours: &[Vec<usize>]) {
    let mut column_of: HashMap<usize, usize> = HashMap::new();
    let mut position: HashMap<usize, usize> = HashMap::new();
    for (column, nodes) in columns.iter().enumerate() {
        for (row, &index) in nodes.iter().enumerate() {
            column_of.insert(index, column);
            position.insert(index, row);
        }
    }
    for sweep in 0..SWEEPS {
        let is_down = sweep % 2 == 0;
        let visit: Vec<usize> = if is_down {
            (1..columns.len()).collect()
        } else {
            (0..columns.len().saturating_sub(1)).rev().collect()
        };
        for column in visit {
            let key = |index: usize| -> f32 {
                let fixed: Vec<usize> = neighbours[index]
                    .iter()
                    .filter(|other| {
                        column_of
                            .get(other)
                            .is_some_and(|c| if is_down { *c < column } else { *c > column })
                    })
                    .map(|other| position[other])
                    .collect();
                if fixed.is_empty() {
                    position[&index] as f32
                } else {
                    fixed.iter().sum::<usize>() as f32 / fixed.len() as f32
                }
            };
            let mut keyed: Vec<(f32, usize)> = columns[column]
                .iter()
                .map(|&index| (key(index), index))
                .collect();
            keyed.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| graph.nodes[a.1].id.cmp(&graph.nodes[b.1].id))
            });
            columns[column] = keyed.into_iter().map(|(_, index)| index).collect();
            for (row, &index) in columns[column].iter().enumerate() {
                position.insert(index, row);
            }
        }
    }
}

struct PlacedRow {
    order: SlotOrder,
    /// How many rows of slots the nodes fill.
    rows: usize,
}

/// Step 5: each config node takes the slot of the column of its first source, or the next free
/// slot to the right of it; past the last slot the row wraps onto a new one, `ROW_PITCH` lower,
/// from slot 0. A node without a source starts at slot 0.
fn place_config_row(
    graph: &TopologyGraph,
    config: &[usize],
    seed: Option<&Seed>,
    metrics: &Metrics,
    pad: f32,
    top: f32,
    rects: &mut [GraphRect],
) -> PlacedRow {
    let mut sources: HashMap<usize, Vec<usize>> = HashMap::new();
    for edge in &graph.edges {
        if graph.nodes[edge.to].kind.placement() == Placement::ConfigRow
            && graph.nodes[edge.from].kind.placement() != Placement::ConfigRow
        {
            sources.entry(edge.to).or_default().push(edge.from);
        }
    }
    let column_of_source = |index: usize| match slot_of(&graph.nodes[index]) {
        Slot::Column(column) => metrics.place_of(column),
        Slot::ConfigRow | Slot::AccessRow => 0,
    };
    let wanted = |index: usize| {
        sources
            .get(&index)
            .and_then(|from| from.first())
            .map_or(0, |&first| column_of_source(first))
    };
    let mean = |index: usize| {
        sources.get(&index).map_or(0., |from| {
            from.iter()
                .map(|&node| column_of_source(node))
                .sum::<usize>() as f32
                / from.len() as f32
        })
    };
    let mut ordered = config.to_vec();
    ordered.sort_by(|&a, &b| {
        let rank = |index: usize| {
            seed.and_then(|seed| seed.rank.get(&graph.nodes[index].id))
                .copied()
                .unwrap_or(usize::MAX)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| mean(a).total_cmp(&mean(b)))
            .then_with(|| graph.nodes[a].id.cmp(&graph.nodes[b].id))
    });
    let mut taken: BTreeSet<(usize, usize)> = BTreeSet::new();
    for &index in &ordered {
        let (row, slot) = free_position(&taken, (0, wanted(index)), metrics.column_count());
        taken.insert((row, slot));
        rects[index] = GraphRect::node(
            metrics.column_x(pad, slot),
            top + row as f32 * ROW_PITCH,
            metrics.node_width,
        );
    }
    PlacedRow {
        order: SlotOrder {
            ids: ids_of(graph, &ordered),
            offset: 0.,
        },
        rows: taken.iter().map(|(row, _)| row + 1).max().unwrap_or(1),
    }
}

/// Step 6, the access row: an account takes the slot of the column of its first source, a binding
/// the slot after its account, a role the slot after its binding; a taken slot moves right, and
/// past the last slot the row wraps like the config row, from the row of the node it follows.
/// Nodes are placed in the order of the previous layout, then accounts, bindings, and roles, so
/// what is new takes a free slot and moves nothing that was there.
fn place_access_row(
    graph: &TopologyGraph,
    access: &[usize],
    seed: Option<&Seed>,
    metrics: &Metrics,
    pad: f32,
    top: f32,
    rects: &mut [GraphRect],
) -> PlacedRow {
    let mut sources: HashMap<usize, Vec<usize>> = HashMap::new();
    for edge in &graph.edges {
        if edge.relation == Relation::Access && graph.nodes[edge.to].kind.is_access() {
            sources.entry(edge.to).or_default().push(edge.from);
        }
    }
    let tier = |index: usize| match graph.nodes[index].kind {
        TopologyKind::RoleBinding | TopologyKind::ClusterRoleBinding => 1,
        TopologyKind::Role | TopologyKind::ClusterRole => 2,
        _ => 0,
    };
    let column_place = |index: usize| match slot_of(&graph.nodes[index]) {
        Slot::Column(column) => Some(metrics.place_of(column)),
        Slot::ConfigRow | Slot::AccessRow => None,
    };
    let mean = |index: usize| {
        let places: Vec<usize> = sources
            .get(&index)
            .into_iter()
            .flatten()
            .filter_map(|&source| column_place(source))
            .collect();
        if places.is_empty() {
            0.
        } else {
            places.iter().sum::<usize>() as f32 / places.len() as f32
        }
    };
    let mut ordered = access.to_vec();
    ordered.sort_by(|&a, &b| {
        let rank = |index: usize| {
            seed.and_then(|seed| seed.rank.get(&graph.nodes[index].id))
                .copied()
                .unwrap_or(usize::MAX)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| tier(a).cmp(&tier(b)))
            .then_with(|| mean(a).total_cmp(&mean(b)))
            .then_with(|| graph.nodes[a].id.cmp(&graph.nodes[b].id))
    });
    let mut taken: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut placed: HashMap<usize, (usize, usize)> = HashMap::new();
    for &index in &ordered {
        let from = sources.get(&index).map_or(&[][..], Vec::as_slice);
        // The first source that is already placed decides: a node that follows an account of the
        // row sits after it, an account sits under the column of its workload.
        let start = from
            .iter()
            .find_map(|source| {
                placed
                    .get(source)
                    .map(|(row, slot)| (*row, slot + 1))
                    .or_else(|| column_place(*source).map(|place| (0, place)))
            })
            .unwrap_or((0, 0));
        let (row, slot) = free_position(&taken, start, metrics.column_count());
        taken.insert((row, slot));
        placed.insert(index, (row, slot));
        rects[index] = GraphRect::node(
            metrics.column_x(pad, slot),
            top + row as f32 * ROW_PITCH,
            metrics.node_width,
        );
    }
    PlacedRow {
        order: SlotOrder {
            ids: ids_of(graph, &ordered),
            offset: 0.,
        },
        rows: taken.iter().map(|(row, _)| row + 1).max().unwrap_or(1),
    }
}

/// The first free `(row, slot)` from `start`, going right and then down.
fn free_position(
    taken: &BTreeSet<(usize, usize)>,
    start: (usize, usize),
    slots: usize,
) -> (usize, usize) {
    let (mut row, mut slot) = (start.0, start.1.min(slots - 1));
    while taken.contains(&(row, slot)) {
        slot += 1;
        if slot == slots {
            row += 1;
            slot = 0;
        }
    }
    (row, slot)
}

#[cfg(test)]
#[path = "topology_layout_tests.rs"]
mod topology_layout_tests;
