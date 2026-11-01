//! Images and diagrams in the document panel: the rows reserved for them in the layout,
//! drawn with halfblocks or a faked protocol on a `TestBackend`, partly scrolled off
//! screen, cached per size, and the text fallbacks without a picker or with an image that
//! does not decode; and choosing the protocol from the environment.

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::Cursor;

use common::{draw, key, paragraphs, press, renderer};
use mrq_core::Document;
use mrq_core::document::DocumentOptions;
use mrq_render::ansi::{ColorLevel, GraphicsProtocol};
use mrq_render::diagram::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer, Registry};
use mrq_tui::graphics::{Errors, Graphics, Picker, ProtocolType};
use mrq_tui::{App, Event};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Color;

const RED: Color = Color::Rgb(255, 0, 0);
const GREEN: Color = Color::Rgb(0, 255, 0);
const BLUE: Color = Color::Rgb(0, 0, 255);

/// A PNG `width` pixels wide of three stripes, 20 pixels each: red, green and blue. At
/// the halfblock picker's 10×20 pixel cells, one row each.
fn stripes(width: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, 60, |_, y| match y / 20 {
        0 => image::Rgba([255, 0, 0, 255]),
        1 => image::Rgba([0, 255, 0, 255]),
        _ => image::Rgba([0, 0, 255, 255]),
    });
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

/// `markdown` as `doc.md` in a new directory, with `pic.png` (20 pixels wide stripes) and
/// `wide.png` (400 pixels wide) next to it.
fn with_images(markdown: &str) -> (tempfile::TempDir, Document) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("pic.png"), stripes(20)).unwrap();
    fs::write(dir.path().join("wide.png"), stripes(400)).unwrap();
    // A PNG header with nothing after it: it has a size, but does not decode.
    fs::write(dir.path().join("broken.png"), &stripes(20)[..33]).unwrap();
    let path = dir.path().join("doc.md");
    fs::write(&path, markdown).unwrap();
    let doc = DocumentOptions::new().load(&path).unwrap();
    (dir, doc)
}

/// A picker for `protocol` without a terminal, with the halfblock picker's 10×20 cells.
fn faked(protocol: ProtocolType) -> Picker {
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(protocol);
    picker
}

/// An app over `doc` with `picker`, sized `width`×`height`, the outline hidden: the
/// document's text starts in column 3 (border and gutter) and row 1.
fn app_with(doc: Document, picker: Option<Picker>, width: u16, height: u16) -> App {
    app_over(App::new(vec![doc], renderer()), picker, width, height)
}

fn app_over(app: App, picker: Option<Picker>, width: u16, height: u16) -> App {
    let mut app = app.with_picker(picker);
    app.handle(Event::Resize(width, height));
    press(&mut app, [key(KeyCode::Char('o'))]);
    app
}

/// `app` drawn on a terminal of its size.
fn buffer(app: &App) -> Buffer {
    let (width, height) = app.size();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| mrq_tui::ui::draw(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The index of the first line of the shown file with a graphic.
fn graphic_line(app: &App) -> usize {
    app.file()
        .lines()
        .iter()
        .position(|line| line.graphic.is_some())
        .unwrap()
}

/// The document rows of `app` drawn, without the title (the temporary path) and the
/// status line.
fn rows(app: &App) -> String {
    let drawn = draw(app);
    let lines: Vec<&str> = drawn.lines().collect();
    lines[1..lines.len() - 2].join("\n")
}

/// The colour that is strongest in both halves of the halfblock at `(column, row)`, which the
/// halfblocks scale to: red, green or blue; `None` for a cell that is not one.
fn halfblock(buffer: &Buffer, column: u16, row: u16) -> Option<Color> {
    let cell = &buffer[(column, row)];
    let strongest = |color: Color| match color {
        Color::Rgb(r, g, b) if r > g && r > b => Some(RED),
        Color::Rgb(r, g, b) if g > r && g > b => Some(GREEN),
        Color::Rgb(r, g, b) if b > r && b > g => Some(BLUE),
        _ => None,
    };
    let upper = strongest(cell.fg)?;
    let halfblock = ["▀", "▄", " "].contains(&cell.symbol());
    (halfblock && strongest(cell.bg) == Some(upper)).then_some(upper)
}

#[test]
fn images_take_their_rows_and_are_drawn_with_halfblocks() {
    let (_dir, doc) = with_images("Before.\n\n![chart](pic.png)\n\nAfter.\n");
    let mut app = app_with(doc, Some(Picker::halfblocks()), 30, 12);
    let at = graphic_line(&app);
    assert_eq!(at, 2);
    let lines = app.file().lines();
    assert_eq!(lines[at].graphic.as_ref().unwrap().rows, 3);
    assert_eq!(lines[6].text(), "After.");
    insta::assert_snapshot!(rows(&app));

    let buffer = buffer(&app);
    let y = 1 + u16::try_from(at).unwrap();
    for (row, color) in [RED, GREEN, BLUE].into_iter().enumerate() {
        let y = y + u16::try_from(row).unwrap();
        assert_eq!(halfblock(&buffer, 3, y), Some(color), "row {row}");
        assert_eq!(halfblock(&buffer, 4, y), Some(color), "row {row}");
    }

    // The image is one block of three lines: the selection steps over it.
    press(&mut app, [key(KeyCode::Down)]);
    let block = app.file().view().selected().unwrap().clone();
    assert_eq!(block.lines, at..at + 3);
    press(&mut app, [key(KeyCode::Down)]);
    assert_eq!(app.file().view().selected().unwrap().lines, 6..7);
}

#[test]
fn an_image_scrolled_partly_off_the_top_shows_its_lower_rows() {
    let markdown = format!("![chart](pic.png)\n\n{}", paragraphs(0..10));
    let (_dir, doc) = with_images(&markdown);
    // Five rows of document: the second paragraph below the image scrolls by two.
    let mut app = app_with(doc, Some(Picker::halfblocks()), 30, 8);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert_eq!(app.file().view().top(), 2);
    insta::assert_snapshot!(rows(&app));
    let buffer = buffer(&app);
    assert_eq!(halfblock(&buffer, 3, 1), Some(BLUE));
    assert_eq!(halfblock(&buffer, 3, 2), None);
}

#[test]
fn an_image_partly_below_the_screen_is_cut_at_the_panel() {
    let (_dir, doc) = with_images("Before.\n\n![chart](pic.png)\n");
    // Three rows of document: the image's first row is the last one.
    let app = app_with(doc, Some(Picker::halfblocks()), 30, 6);
    insta::assert_snapshot!(rows(&app));
    let buffer = buffer(&app);
    assert_eq!(halfblock(&buffer, 3, 3), Some(RED));
    // The border and the status line are left alone.
    assert_eq!(buffer[(3, 4)].symbol(), "━");
    assert_eq!(halfblock(&buffer, 3, 5), None);
}

/// The rows of the screen with a kitty placeholder, and whether the image was
/// transmitted on the way.
fn kitty_rows(buffer: &Buffer) -> (Vec<u16>, bool) {
    let mut rows = Vec::new();
    let mut transmitted = false;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            transmitted |= symbol.contains("\x1b_G");
            if symbol.contains('\u{10EEEE}') && rows.last() != Some(&y) {
                rows.push(y);
            }
        }
    }
    (rows, transmitted)
}

#[test]
fn a_kitty_image_is_placed_on_its_visible_rows_only() {
    let markdown = format!("![chart](pic.png)\n\n{}", paragraphs(0..10));
    let (_dir, doc) = with_images(&markdown);
    let mut app = app_with(doc, Some(faked(ProtocolType::Kitty)), 30, 8);
    let (rows, transmitted) = kitty_rows(&buffer(&app));
    assert_eq!(rows, [1, 2, 3]);
    assert!(transmitted);
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    // Sent once: the cached image is only placed again.
    let (rows, transmitted) = kitty_rows(&buffer(&app));
    assert_eq!(rows, [1]);
    assert!(!transmitted);
    // Scrolled off the screen, it is not placed at all.
    press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
    assert!(app.file().view().top() >= 3);
    assert_eq!(kitty_rows(&buffer(&app)).0, Vec::<u16>::new());
}

#[test]
fn sixel_and_iterm2_images_are_drawn_partly_scrolled() {
    for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
        let markdown = format!("![chart](pic.png)\n\n{}", paragraphs(0..10));
        let (_dir, doc) = with_images(&markdown);
        let mut app = app_with(doc, Some(faked(protocol)), 30, 8);
        let escapes = |buffer: &Buffer| {
            (0..buffer.area.height)
                .filter(|&y| {
                    (0..buffer.area.width).any(|x| buffer[(x, y)].symbol().contains('\x1b'))
                })
                .collect::<Vec<_>>()
        };
        assert!(!escapes(&buffer(&app)).is_empty(), "{protocol:?}");
        press(&mut app, [key(KeyCode::Down), key(KeyCode::Down)]);
        // Only the last row is on screen; nothing is drawn outside the panel.
        let rows = escapes(&buffer(&app));
        assert_eq!(rows, [1], "{protocol:?}");
    }
}

#[test]
fn the_cell_size_comes_from_the_picker() {
    let (_dir, doc) = with_images("![chart](pic.png)\n");
    #[allow(deprecated, reason = "the one way to give a picker a font size")]
    let picker = Picker::from_fontsize((5, 10).into());
    let app = app_with(doc, Some(picker), 30, 12);
    let graphic = app.file().lines()[0].graphic.clone().unwrap();
    assert_eq!((graphic.columns, graphic.rows), (4, 6));
    assert_eq!(app.file().view().blocks()[0].lines, 0..6);
}

#[test]
fn images_are_decoded_and_scaled_once_per_size() {
    let (_dir, doc) = with_images("![chart](wide.png)\n");
    let mut app = app_with(doc, Some(Picker::halfblocks()), 60, 12);
    assert_eq!(
        app.cached_images(),
        0,
        "nothing is decoded before it is drawn"
    );
    buffer(&app);
    buffer(&app);
    assert_eq!(app.cached_images(), 1);
    // Narrower than the image: scaled down, a second size.
    app.handle(Event::Resize(30, 12));
    buffer(&app);
    assert_eq!(app.cached_images(), 2);
    app.handle(Event::Resize(60, 12));
    buffer(&app);
    assert_eq!(app.cached_images(), 2);
    // Another picker draws everything afresh.
    app.set_picker(Some(faked(ProtocolType::Kitty)));
    assert_eq!(app.cached_images(), 0);
}

#[test]
fn without_a_picker_images_show_their_alt_text() {
    let (_dir, doc) = with_images("Before.\n\n![chart](pic.png)\n\nAfter.\n");
    let app = app_with(doc, None, 30, 8);
    assert!(app.file().lines().iter().all(|line| line.graphic.is_none()));
    insta::assert_snapshot!(rows(&app));
    // Taking the picker away lays the document out again without graphics.
    let (_dir, doc) = with_images("![chart](pic.png)\n\nAfter.\n");
    let mut app = app_with(doc, Some(Picker::halfblocks()), 30, 8);
    app.set_picker(None);
    assert_eq!(app.file().lines()[0].text(), "[image: chart]");
    assert_eq!(app.file().lines()[2].text(), "After.");
}

#[test]
fn an_image_that_does_not_decode_shows_its_alt_text_in_its_rows() {
    let (_dir, doc) = with_images("![chart](broken.png)\n\nAfter.\n");
    let app = app_with(doc, Some(Picker::halfblocks()), 30, 8);
    assert_eq!(app.file().lines()[0].graphic.as_ref().unwrap().rows, 3);
    insta::assert_snapshot!(rows(&app));
}

/// A diagram renderer that answers from memory.
struct Fixed {
    languages: Vec<String>,
    formats: Vec<DiagramFormat>,
    output: DiagramOutput,
}

impl Fixed {
    fn new(output: DiagramOutput) -> Self {
        Self {
            languages: vec!["bob".to_owned()],
            formats: vec![output.format()],
            output,
        }
    }
}

impl DiagramRenderer for Fixed {
    fn id(&self) -> &'static str {
        "fixed"
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &self.formats
    }

    fn render(
        &self,
        _lang: &str,
        _src: &str,
        _format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        Ok(self.output.clone())
    }
}

/// An app over a `bob` diagram, rendered as PNG stripes or else as text.
fn diagram_app(picker: Option<Picker>) -> App {
    let registry = Registry::new()
        .with_renderer(Fixed::new(DiagramOutput::Png(stripes(20))))
        .with_renderer(Fixed::new(DiagramOutput::Text(
            "+--+\n|ok|\n+--+\n".to_owned(),
        )));
    let doc = Document::parse("```bob\nsource\n```\n\nAfter.\n");
    let app = App::new(vec![doc], renderer().with_diagrams(registry));
    app_over(app, picker, 30, 9)
}

#[test]
fn diagrams_are_drawn_as_images() {
    let app = diagram_app(Some(Picker::halfblocks()));
    assert_eq!(app.file().lines()[0].graphic.as_ref().unwrap().rows, 3);
    insta::assert_snapshot!(rows(&app));
    assert_eq!(halfblock(&buffer(&app), 3, 3), Some(BLUE));
}

#[test]
fn without_a_picker_diagrams_show_their_text() {
    let app = diagram_app(None);
    insta::assert_snapshot!(rows(&app));
}

fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let vars: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| vars.get(name).cloned()
}

#[test]
fn graphics_are_chosen_from_the_environment() {
    let choose = |vars: &[(&str, &str)]| Graphics::from_env(ColorLevel::TrueColor, env(vars));
    let detect = Graphics::Detect { halfblocks: true };
    assert_eq!(choose(&[("TERM", "xterm-kitty")]), detect);
    assert_eq!(choose(&[("MRQ_GRAPHICS", "none")]), Graphics::Off);
    assert_eq!(choose(&[("MRQ_GRAPHICS", "bogus")]), Graphics::Off);
    assert_eq!(
        choose(&[("MRQ_GRAPHICS", "halfblocks")]),
        Graphics::Halfblocks
    );
    for (name, protocol) in [
        ("kitty", GraphicsProtocol::Kitty),
        ("iTerm2", GraphicsProtocol::Iterm2),
        ("sixel", GraphicsProtocol::Sixel),
    ] {
        assert_eq!(
            choose(&[("MRQ_GRAPHICS", name)]),
            Graphics::Protocol(protocol)
        );
    }
    assert_eq!(choose(&[("TERM", "dumb")]), Graphics::Off);
    // tmux and screen drop the escapes; halfblocks are text.
    assert_eq!(choose(&[("TMUX", "/tmp/tmux")]), Graphics::Halfblocks);
    assert_eq!(choose(&[("TERM", "screen-256color")]), Graphics::Halfblocks);
    assert_eq!(
        choose(&[("TMUX", "/tmp/tmux"), ("MRQ_GRAPHICS", "kitty")]),
        Graphics::Halfblocks
    );
    assert_eq!(
        choose(&[("TMUX", "/tmp/tmux"), ("MRQ_GRAPHICS", "none")]),
        Graphics::Off
    );
    // Halfblocks are colours, so not without them; a protocol still shows images.
    let plain = |vars: &[(&str, &str)]| Graphics::from_env(ColorLevel::None, env(vars));
    assert_eq!(plain(&[]), Graphics::Detect { halfblocks: false });
    assert_eq!(plain(&[("TMUX", "/tmp/tmux")]), Graphics::Off);
    assert_eq!(
        plain(&[("MRQ_GRAPHICS", "kitty")]),
        Graphics::Protocol(GraphicsProtocol::Kitty)
    );
}

#[test]
fn the_picker_comes_from_the_query_or_falls_back_to_halfblocks() {
    let unused = || -> Result<Picker, Errors> { panic!("the terminal is not queried") };
    let failed = || -> Result<Picker, Errors> { Err(Errors::NoStdinResponse) };
    let answered = |protocol| move || -> Result<Picker, Errors> { Ok(faked(protocol)) };
    let protocol = |picker: Option<Picker>| picker.map(|picker| picker.protocol_type());

    assert!(Graphics::Off.picker(unused).is_none());
    assert_eq!(
        protocol(Graphics::Halfblocks.picker(unused)),
        Some(ProtocolType::Halfblocks)
    );
    let detect = Graphics::Detect { halfblocks: true };
    assert_eq!(
        protocol(detect.picker(answered(ProtocolType::Kitty))),
        Some(ProtocolType::Kitty)
    );
    assert_eq!(
        protocol(detect.picker(answered(ProtocolType::Halfblocks))),
        Some(ProtocolType::Halfblocks)
    );
    assert_eq!(
        protocol(detect.picker(failed)),
        Some(ProtocolType::Halfblocks)
    );
    let colourless = Graphics::Detect { halfblocks: false };
    assert_eq!(
        protocol(colourless.picker(answered(ProtocolType::Iterm2))),
        Some(ProtocolType::Iterm2)
    );
    assert!(
        colourless
            .picker(answered(ProtocolType::Halfblocks))
            .is_none()
    );
    assert!(colourless.picker(failed).is_none());

    // A forced protocol keeps the font size the terminal answered with.
    let forced = Graphics::Protocol(GraphicsProtocol::Sixel);
    let picker = forced
        .picker(|| {
            #[allow(deprecated, reason = "the one way to give a picker a font size")]
            Ok(Picker::from_fontsize((7, 14).into()))
        })
        .unwrap();
    assert_eq!(picker.protocol_type(), ProtocolType::Sixel);
    assert_eq!(
        (picker.font_size().width, picker.font_size().height),
        (7, 14)
    );
    assert_eq!(protocol(forced.picker(failed)), Some(ProtocolType::Sixel));
}
