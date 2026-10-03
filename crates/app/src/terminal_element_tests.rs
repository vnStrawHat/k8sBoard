use futures::channel::mpsc;
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, ParentElement as _, Point, Render, TestAppContext,
    WindowBounds, WindowHandle, WindowOptions, div, px,
};
use oneterm_vt::{Pos, Size as VtSize};

use super::*;

fn cells(width: f32, height: f32) -> Size<Pixels> {
    size(px(width), px(height))
}

#[test]
fn grid_size_floors_and_keeps_a_minimum() {
    assert_eq!(
        grid_size(cells(805., 410.), cells(8., 17.)),
        GridSize {
            cols: 100,
            rows: 24
        }
    );
    assert_eq!(
        grid_size(cells(3., 2.), cells(8., 17.)),
        GridSize { cols: 2, rows: 1 }
    );
    // A cell that could not be measured must not divide by zero.
    assert_eq!(
        grid_size(cells(805., 410.), cells(0., 0.)),
        GridSize { cols: 2, rows: 1 }
    );
}

fn range(start: (u64, u16), end: (u64, u16), is_block: bool) -> SelectionRange {
    SelectionRange {
        start: Pos {
            row: RowId(start.0),
            col: start.1,
        },
        end: Pos {
            row: RowId(end.0),
            col: end.1,
        },
        is_block,
    }
}

fn span(row: u16, start_col: u16, end_col: u16) -> SelectionSpan {
    SelectionSpan {
        row,
        start_col,
        end_col,
    }
}

const VIEW: VtSize = VtSize { rows: 4, cols: 10 };

#[test]
fn a_selection_inside_one_row_covers_its_cells_inclusively() {
    let spans = selection_spans(range((101, 2), (101, 4), false), RowId(100), VIEW);
    assert_eq!(spans, [span(1, 2, 5)]);
}

#[test]
fn a_stream_selection_takes_the_rest_of_its_first_row_and_the_start_of_its_last() {
    let spans = selection_spans(range((100, 6), (102, 3), false), RowId(100), VIEW);
    assert_eq!(spans, [span(0, 6, 10), span(1, 0, 10), span(2, 0, 4)]);
}

#[test]
fn a_block_selection_takes_the_same_columns_on_every_row() {
    let spans = selection_spans(range((100, 7), (101, 3), true), RowId(100), VIEW);
    assert_eq!(spans, [span(0, 3, 8), span(1, 3, 8)]);
}

#[test]
fn a_selection_is_clipped_to_the_visible_rows() {
    // Starts in history, ends below the screen.
    let spans = selection_spans(range((90, 5), (200, 1), false), RowId(100), VIEW);
    assert_eq!(
        spans,
        [
            span(0, 0, 10),
            span(1, 0, 10),
            span(2, 0, 10),
            span(3, 0, 10)
        ]
    );
    // Entirely above the viewport.
    assert!(selection_spans(range((10, 0), (20, 4), false), RowId(100), VIEW).is_empty());
}

/// A window with one terminal element, standing in for the shell tab of a later step.
struct Probe {
    session: Rc<RefCell<TerminalSession>>,
    input: UnboundedSender<ShellInput>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(terminal_element(
            Rc::clone(&self.session),
            self.input.clone(),
        ))
    }
}

struct Fixture {
    window: WindowHandle<Root>,
    _probe: Entity<Probe>,
    session: Rc<RefCell<TerminalSession>>,
    resizes: mpsc::UnboundedReceiver<ShellInput>,
}

fn open_probe(width: f32, height: f32, cx: &mut TestAppContext) -> Fixture {
    let session = Rc::new(RefCell::new(TerminalSession::new(GridSize {
        cols: 80,
        rows: 24,
    })));
    let (input, resizes) = mpsc::unbounded();
    let probe_session = Rc::clone(&session);
    let (window, probe) = cx.update(|cx| {
        gpui_kit::init(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(width), px(height)),
        };
        let (window, probe) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|_| Probe {
                    session: probe_session,
                    input,
                })
            },
        )
        .expect("open the test window");
        (window.downcast::<Root>().expect("a Root window"), probe)
    });
    Fixture {
        window,
        _probe: probe,
        session,
        resizes,
    }
}

fn render(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx)
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn queued_resizes(fixture: &mut Fixture) -> Vec<GridSize> {
    let mut sizes = Vec::new();
    while let Ok(input) = fixture.resizes.try_recv() {
        match input {
            ShellInput::Resize(size) => sizes.push(size),
            ShellInput::Bytes(_) => panic!("the element sends no bytes"),
        }
    }
    sizes
}

#[gpui_kit::test]
fn the_element_sizes_the_grid_to_its_bounds_and_queues_one_resize(cx: &mut TestAppContext) {
    let mut fixture = open_probe(640., 400., cx);
    fixture
        .session
        .borrow_mut()
        .feed(b"hello\r\nworld", Instant::now());
    render(&fixture, cx);
    let queued = queued_resizes(&mut fixture);
    assert_eq!(queued.len(), 1, "one resize for the first measured size");
    let grid = queued[0];
    assert!(grid.cols >= 2 && grid.rows >= 1);
    let snapshot_size = fixture.session.borrow_mut().snapshot(Instant::now()).size();
    assert_eq!(
        (snapshot_size.cols, snapshot_size.rows),
        (grid.cols, grid.rows)
    );

    // Another frame at the same size asks for nothing.
    render(&fixture, cx);
    assert!(queued_resizes(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn the_element_paints_fed_text_and_the_theme_palette(cx: &mut TestAppContext) {
    let mut fixture = open_probe(640., 400., cx);
    fixture.session.borrow_mut().feed(
        b"\x1b[31mred\x1b[0m \x1b[1;4mbold\x1b[0m \xe4\xbd\xa0\xe5\xa5\xbd\r\n",
        Instant::now(),
    );
    render(&fixture, cx);
    queued_resizes(&mut fixture);
    let mut session = fixture.session.borrow_mut();
    // The frame installed the theme palette: the default colors are the theme's, not the engine's.
    let theme_background = cx.update(|cx| {
        use gpui_kit::component::ActiveTheme as _;
        rgb_of_theme_background(cx.theme())
    });
    assert_eq!(session.palette().background, theme_background);
    let snapshot = session.snapshot(Instant::now());
    assert!(!snapshot.rows().is_empty());
}

fn rgb_of_theme_background(theme: &gpui_kit::component::ThemeColor) -> Rgb {
    crate::terminal_session::rgb_of(theme.background)
}

#[gpui_kit::test]
fn a_session_held_by_its_owner_skips_the_frame(cx: &mut TestAppContext) {
    let mut fixture = open_probe(640., 400., cx);
    render(&fixture, cx);
    let measured = queued_resizes(&mut fixture);
    assert_eq!(measured.len(), 1, "the first frame measured the window");
    // Make the grid differ from the window, then hold the session across a frame.
    fixture
        .session
        .borrow_mut()
        .resize(GridSize { cols: 10, rows: 5 });
    let held = fixture.session.borrow_mut();
    render(&fixture, cx);
    drop(held);
    assert!(
        queued_resizes(&mut fixture).is_empty(),
        "a frame that cannot reach the session measures nothing"
    );
    // The next frame, with the session free again, resizes as usual.
    render(&fixture, cx);
    assert_eq!(queued_resizes(&mut fixture), measured);
}
