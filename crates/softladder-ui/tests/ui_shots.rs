//! Headless UI screenshots.
//!
//! This machine has no display server, so the editor is reviewed by rendering a
//! real frame — the same `EditorApp::draw` the window uses — into a PNG with a
//! small software rasterizer: egui tessellates its shapes into triangles, the
//! font atlas arrives through `TexturesDelta`, and this file walks the triangles
//! and blends them. No GPU, no window, nothing added to the shipped binary.
//!
//! Run it with `cargo test -p softladder-ui --test ui_shots` and look in
//! `target/ui-shots/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::epaint::{ClippedPrimitive, ImageData, Primitive, TextureId};
use egui::{Context, RawInput, Rect, Vec2};
use softladder_ui::app::{CentreTab, EditorApp};
use softladder_ui::Theme;

/// One decoded texture: premultiplied linear RGBA in `0..=1`.
struct Texture {
    size: [usize; 2],
    pixels: Vec<[f32; 4]>,
}

impl Texture {
    fn sample(&self, uv: egui::Pos2) -> [f32; 4] {
        let width = self.size[0];
        let height = self.size[1];
        if width == 0 || height == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        let x = (uv.x * width as f32).floor().clamp(0.0, (width - 1) as f32) as usize;
        let y = (uv.y * height as f32)
            .floor()
            .clamp(0.0, (height - 1) as f32) as usize;
        self.pixels
            .get(y * width + x)
            .copied()
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    }
}

/// The textures egui has uploaded so far (in practice: the font atlas).
#[derive(Default)]
struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, image) in &delta.set {
            let source: Vec<[f32; 4]> = match &image.image {
                ImageData::Color(color) => color
                    .pixels
                    .iter()
                    .map(|pixel| {
                        let [r, g, b, a] = pixel.to_array();
                        [
                            f32::from(r) / 255.0,
                            f32::from(g) / 255.0,
                            f32::from(b) / 255.0,
                            f32::from(a) / 255.0,
                        ]
                    })
                    .collect(),
                // `srgba_pixels` applies egui's own coverage gamma and yields the
                // premultiplied colour the shader multiplies with.
                ImageData::Font(font) => font
                    .srgba_pixels(None)
                    .map(|pixel| {
                        let [r, g, b, a] = pixel.to_array();
                        [
                            f32::from(r) / 255.0,
                            f32::from(g) / 255.0,
                            f32::from(b) / 255.0,
                            f32::from(a) / 255.0,
                        ]
                    })
                    .collect(),
            };
            let size = match &image.image {
                ImageData::Color(color) => color.size,
                ImageData::Font(font) => font.size,
            };
            match image.pos {
                // A partial update patches the existing texture.
                Some([x, y]) => {
                    if let Some(texture) = self.map.get_mut(id) {
                        for row in 0..size[1] {
                            for column in 0..size[0] {
                                let target = (y + row) * texture.size[0] + (x + column);
                                let source_index = row * size[0] + column;
                                if let (Some(slot), Some(value)) =
                                    (texture.pixels.get_mut(target), source.get(source_index))
                                {
                                    *slot = *value;
                                }
                            }
                        }
                    }
                }
                None => {
                    self.map.insert(
                        *id,
                        Texture {
                            size,
                            pixels: source,
                        },
                    );
                }
            }
        }
        for id in &delta.free {
            self.map.remove(id);
        }
    }
}

/// Blends the tessellated triangles of one frame into an image.
fn rasterize(
    primitives: &[ClippedPrimitive],
    textures: &Textures,
    size: [usize; 2],
    pixels_per_point: f32,
) -> Vec<u8> {
    let width = size[0];
    let height = size[1];
    // egui's dark background, so anything the panels do not paint is not black.
    let mut buffer = vec![[0.0f32, 0.0, 0.0, 1.0]; width * height];
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.map.get(&mesh.texture_id) else {
            continue;
        };
        let clip = primitive.clip_rect;
        let min_x = (clip.min.x * pixels_per_point).floor().max(0.0) as usize;
        let min_y = (clip.min.y * pixels_per_point).floor().max(0.0) as usize;
        let max_x = ((clip.max.x * pixels_per_point).ceil() as usize).min(width);
        let max_y = ((clip.max.y * pixels_per_point).ceil() as usize).min(height);

        for triangle in mesh.indices.chunks_exact(3) {
            let Some(vertices) = triangle
                .iter()
                .map(|index| mesh.vertices.get(*index as usize))
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let positions: Vec<[f32; 2]> = vertices
                .iter()
                .map(|vertex| {
                    [
                        vertex.pos.x * pixels_per_point,
                        vertex.pos.y * pixels_per_point,
                    ]
                })
                .collect();
            let area = (positions[1][0] - positions[0][0]) * (positions[2][1] - positions[0][1])
                - (positions[2][0] - positions[0][0]) * (positions[1][1] - positions[0][1]);
            if area.abs() < 1e-6 {
                continue;
            }
            let inverse_area = 1.0 / area;
            let triangle_min_x = positions
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(min_x as f32) as usize;
            let triangle_max_x = (positions
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(max_x);
            let triangle_min_y = positions
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(min_y as f32) as usize;
            let triangle_max_y = (positions
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(max_y);

            for y in triangle_min_y..triangle_max_y {
                for x in triangle_min_x..triangle_max_x {
                    let px = x as f32 + 0.5;
                    let py = y as f32 + 0.5;
                    // Barycentric weights.
                    let w0 = ((positions[1][0] - px) * (positions[2][1] - py)
                        - (positions[2][0] - px) * (positions[1][1] - py))
                        * inverse_area;
                    let w1 = ((positions[2][0] - px) * (positions[0][1] - py)
                        - (positions[0][0] - px) * (positions[2][1] - py))
                        * inverse_area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -0.0005 || w1 < -0.0005 || w2 < -0.0005 {
                        continue;
                    }
                    let mut color = [0.0f32; 4];
                    let mut uv = [0.0f32; 2];
                    for (weight, vertex) in [w0, w1, w2].iter().zip(&vertices) {
                        let [r, g, b, a] = vertex.color.to_array();
                        color[0] += weight * f32::from(r) / 255.0;
                        color[1] += weight * f32::from(g) / 255.0;
                        color[2] += weight * f32::from(b) / 255.0;
                        color[3] += weight * f32::from(a) / 255.0;
                        uv[0] += weight * vertex.uv.x;
                        uv[1] += weight * vertex.uv.y;
                    }
                    let texel = texture.sample(egui::pos2(uv[0], uv[1]));
                    // Premultiplied source over the destination.
                    let source = [
                        color[0] * texel[0],
                        color[1] * texel[1],
                        color[2] * texel[2],
                        color[3] * texel[3],
                    ];
                    if let Some(pixel) = buffer.get_mut(y * width + x) {
                        let keep = 1.0 - source[3];
                        pixel[0] = source[0] + pixel[0] * keep;
                        pixel[1] = source[1] + pixel[1] * keep;
                        pixel[2] = source[2] + pixel[2] * keep;
                        pixel[3] = source[3] + pixel[3] * keep;
                    }
                }
            }
        }
    }

    let mut bytes = Vec::with_capacity(width * height * 4);
    for pixel in buffer {
        // PNG wants straight alpha; the window is opaque anyway.
        let alpha = pixel[3].max(1e-4);
        for channel in &pixel[..3] {
            bytes.push((channel / alpha).clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8);
        }
        bytes.push((pixel[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }
    bytes
}

/// Renders `app` at `size` and writes a PNG.
fn shoot(app: &mut EditorApp, size: Vec2, path: &Path) {
    let ctx = Context::default();
    let mut textures = Textures::default();
    let pixels_per_point = 1.0;
    let input = || RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..RawInput::default()
    };
    // Two frames: the first builds the font atlas and lays the panels out.
    let first = ctx.run(input(), |ctx| app.draw(ctx));
    textures.apply(&first.textures_delta);
    let second = ctx.run(input(), |ctx| app.draw(ctx));
    textures.apply(&second.textures_delta);
    let primitives = ctx.tessellate(second.shapes, pixels_per_point);
    let bytes = rasterize(
        &primitives,
        &textures,
        [size.x as usize, size.y as usize],
        pixels_per_point,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the output directory is creatable");
    }
    let image = image::RgbaImage::from_raw(size.x as u32, size.y as u32, bytes)
        .expect("the buffer matches the image size");
    image.save(path).expect("the screenshot is written");
    println!("wrote {}", path.display());
}

fn project() -> softladder_core::Project {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/traffic_light.slprj");
    softladder_project::native::load(&path).expect("the shipped example loads")
}

fn output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots")
}

/// A contact, a coil and a timer on one rung, so the canvas has content.
fn rich_project() -> softladder_core::Project {
    use softladder_core::{ElementKind, PlacedElement, Rung, Symbol, TimerMode, VarKind, VarRef};
    let var = |text: &str| text.parse::<VarRef>().expect("variable parses");
    let mut project = project();
    let mut rung = Rung::new(90);
    rung.label = "conveyor".to_owned();
    rung.comment = "Start/stop with a 3 s run-on delay".to_owned();
    rung.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        var("%I0"),
        0,
        0,
    ));
    rung.elements.push(PlacedElement::with_var(
        ElementKind::ContactNc,
        var("%I2"),
        1,
        0,
    ));
    rung.elements.push(PlacedElement::with_params(
        ElementKind::Timer {
            mode: TimerMode::On,
        },
        3,
        0,
        &["3000"],
    ));
    rung.elements.last_mut().expect("just pushed").var = Some(VarRef::new(VarKind::TimerIec, 5));
    rung.elements.push(PlacedElement::with_var(
        ElementKind::CoilOut,
        var("%Q3"),
        4,
        0,
    ));
    project.rungs.push(rung);
    if let Some(section) = project.sections.first_mut() {
        section.rungs.push(90);
    }
    project.symbols.push(Symbol {
        name: "conveyor_run".to_owned(),
        var: Some(var("%Q3")),
        comment: "Conveyor motor contactor".to_owned(),
        unit: None,
    });
    project
}

/// A project with diagnostics of every severity, so the Problems document has
/// rows to draw. Everything is built through the public model, exactly like an
/// imported project would be.
fn broken_project() -> softladder_core::Project {
    use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section, VarRef};

    let var = |text: &str| text.parse::<VarRef>().expect("variable parses");
    let mut project = Project::new("diagnostics");
    project.schema_version = 2;

    // A rung with a contact that has no variable: `SL-E004`.
    let mut orphan = Rung {
        id: 1,
        label: "unbound".to_owned(),
        comment: "The element under the cursor has no tag".to_owned(),
        ..Rung::new(1)
    };
    orphan.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        var("%I0"),
        0,
        0,
    ));
    orphan
        .elements
        .push(PlacedElement::new(ElementKind::ContactNo, 1, 0));
    orphan.elements.push(PlacedElement::with_var(
        ElementKind::CoilOut,
        var("%Q0"),
        2,
        0,
    ));

    // A rung whose live row has no cell in column 0: `SL-W001`.
    let mut floating = Rung {
        id: 2,
        label: "floating row".to_owned(),
        comment: String::new(),
        ..Rung::new(2)
    };
    floating.elements.push(PlacedElement::with_var(
        ElementKind::ContactNo,
        var("%I1"),
        2,
        1,
    ));

    project.rungs.push(orphan);
    project.rungs.push(floating);
    let mut section = Section::new(1, "Main");
    section.rungs.push(1);
    section.rungs.push(2);
    section.rungs.push(404); // names a rung the project does not define
    project.sections.push(section);
    project
}

/// A project with a ladder section and a sequential chart, for the SFC shots.
///
/// The chart is a small conveyor sequence: an initial step, an AND divergence
/// into two branches, a merge and the loop back, so the shots show the doubled
/// transition bar, an OR junction and the wiring the editor derives from the
/// model's `from`/`to` sets.
fn sfc_project() -> softladder_core::Project {
    use softladder_core::{
        Expr, Project, Section, SequentialPage, Step, Symbol, Transition, VarRef,
    };
    let var = |text: &str| text.parse::<VarRef>().expect("variable parses");
    let condition = |text: &str| text.parse::<Expr>().expect("condition parses");

    let mut project = project();
    let mut page = SequentialPage::new(0, "Conveyor sequence: start, run, stop");
    page.steps.push(Step {
        number: 0,
        is_initial: true,
        x: 0,
        y: 0,
        page: 0,
    });
    page.steps.push(Step {
        number: 1,
        is_initial: false,
        x: 0,
        y: 2,
        page: 0,
    });
    page.steps.push(Step {
        number: 2,
        is_initial: false,
        x: 2,
        y: 2,
        page: 0,
    });
    page.steps.push(Step {
        number: 3,
        is_initial: false,
        x: 1,
        y: 4,
        page: 0,
    });
    page.transitions.push(Transition {
        number: 0,
        condition: Some(condition("%I0")),
        from: vec![0],
        to: vec![1, 2],
        page: 0,
        x: 0,
        y: 1,
    });
    page.transitions.push(Transition {
        number: 1,
        condition: Some(condition("%I1")),
        from: vec![1],
        to: vec![3],
        page: 0,
        x: 0,
        y: 3,
    });
    page.transitions.push(Transition {
        number: 2,
        condition: Some(condition("%I2 AND %I3")),
        from: vec![2],
        to: vec![3],
        page: 0,
        x: 2,
        y: 3,
    });
    page.transitions.push(Transition {
        number: 3,
        condition: Some(condition("%M0")),
        from: vec![3],
        to: vec![0],
        page: 0,
        x: 1,
        y: 5,
    });
    project.sections.push(Section::sfc(2, "Conveyor", page));
    project.symbols.push(Symbol {
        name: "cycle_done".to_owned(),
        var: Some(var("%M0")),
        comment: "One conveyor cycle finished".to_owned(),
        unit: None,
    });
    let _ = Project::new("unused");
    project
}

/// A chart broken every way the sequential lint reports.
///
/// `SL-W011` (a transition with no condition), `SL-E011` (a transition naming a
/// step the page does not define), `SL-W001` (a step no transition can activate)
/// and `SL-W002` (a section with no page) all land in the Problems document, so
/// the shot covers every code and the location each one carries.
fn broken_sfc_project() -> softladder_core::Project {
    use softladder_core::{
        ElementKind, PlacedElement, Project, Rung, Section, SequentialPage, Step, Transition,
        VarRef,
    };
    let var = |text: &str| text.parse::<VarRef>().expect("variable parses");
    let mut project = Project::new("broken chart");
    project.schema_version = 2;
    let mut main = Section::new(1, "Main");
    main.rungs.push(1);
    project.sections.push(main);
    project.rungs.push(Rung {
        id: 1,
        label: "start_stop".to_owned(),
        elements: vec![PlacedElement::with_var(
            ElementKind::ContactNo,
            var("%I0"),
            0,
            0,
        )],
        ..Rung::new(1)
    });

    let mut page = SequentialPage::new(0, "start-up");
    page.steps.push(Step {
        number: 0,
        is_initial: true,
        x: 0,
        y: 0,
        page: 0,
    });
    page.steps.push(Step {
        number: 1,
        is_initial: false,
        x: 0,
        y: 2,
        page: 0,
    });
    // A step no transition can activate: `SL-W001`.
    page.steps.push(Step {
        number: 2,
        is_initial: false,
        x: 2,
        y: 0,
        page: 0,
    });
    // An unconditional transition, which fires whenever its source is active:
    // `SL-W011`.
    page.transitions.push(Transition {
        number: 0,
        condition: None,
        from: vec![0],
        to: vec![1],
        page: 0,
        x: 0,
        y: 1,
    });
    // A transition naming a step that does not exist: `SL-E011`.
    page.transitions.push(Transition {
        number: 1,
        condition: Some("%I0".parse().expect("a condition parses")),
        from: vec![1],
        to: vec![9],
        page: 0,
        x: 0,
        y: 3,
    });
    project.sections.push(Section::sfc(2, "Conveyor", page));

    // A sequential section whose page has not been drawn: `SL-W002`.
    let mut empty = Section::new(3, "Manual");
    empty.language = softladder_core::SectionLanguage::Sfc;
    project.sections.push(empty);
    project
}

#[test]
fn shoot_the_editor() {
    let dir = output_dir();
    let size = Vec2::new(1440.0, 900.0);

    // 1. The example project as it opens.
    let mut app = EditorApp::new(project());
    shoot(&mut app, size, &dir.join("01-open.png"));

    // 2. A richer rung, with something selected so the property strip shows.
    let mut app = EditorApp::new(rich_project());
    let rung = app.project().rungs.last().expect("a rung").id;
    app.select(rung, Some((1, 0)));
    shoot(&mut app, size, &dir.join("02-selected.png"));

    // 3. Running, on the ladder: the live state the whole tool exists to show.
    let mut app = EditorApp::new(rich_project());
    // Close the start button and leave the stop button open, so the running
    // ladder has an energised path to draw.
    for (text, value) in [("%I0", true), ("%I2", false), ("%I1", true)] {
        let var: softladder_core::VarRef = text.parse().expect("a valid variable");
        app.set_variable(&var, softladder_core::Value::Bit(value))
            .expect("the store accepts inputs");
    }
    app.handle(softladder_ui::shortcuts::Action::RunStop);
    for _ in 0..40 {
        app.single_scan();
    }
    shoot(&mut app, size, &dir.join("03-running.png"));

    // 3b. The bench document, running: the operator screen.
    let mut app = EditorApp::new(rich_project());
    app.handle(softladder_ui::shortcuts::Action::AutoFillBench);
    app.show_document(CentreTab::Bench);
    app.handle(softladder_ui::shortcuts::Action::RunStop);
    for _ in 0..40 {
        app.single_scan();
    }
    shoot(&mut app, size, &dir.join("03b-bench.png"));

    // 4. While placing an element from the palette.
    let mut app = EditorApp::new(rich_project());
    app.handle(softladder_ui::shortcuts::Action::Pick(
        softladder_core::ElementKind::ContactNc,
    ));
    shoot(&mut app, size, &dir.join("04-placing.png"));

    // 5. The diagnostics list, with real problems: an element with no variable
    // (`SL-E004`), a live row with no path to the rail (`SL-W001`) and a
    // section that names a rung which does not exist (`SL-E011`).
    let mut app = EditorApp::new(broken_project());
    assert!(
        app.editor().problems().len() >= 2,
        "the broken project must produce diagnostics for the Problems document: {:?}",
        app.editor().problems()
    );
    app.show_document(CentreTab::Problems);
    shoot(&mut app, size, &dir.join("05-problems.png"));

    // 6. The watch table with rows in it: live values, a format, a modify field
    // and a force.
    let mut app = EditorApp::new(rich_project());
    // The watch document owns its rows, so the harness queues them through the
    // panel's own entry point rather than reaching into the editor.
    softladder_ui::panels::watch::preload(
        ["%I0", "%I2", "%MW10", "%TM5.Q"]
            .iter()
            .map(|text| {
                let var: softladder_core::VarRef = text.parse().expect("a valid variable");
                let mut row = softladder_ui::WatchRow::new(var);
                if text.starts_with("%MW") || text.starts_with("%TM") {
                    row.format = softladder_ui::ValueFormat::Signed;
                }
                row
            })
            .collect(),
        vec![("%MW10".parse().expect("a valid variable"), true)],
    );
    app.show_document(CentreTab::Watch);
    shoot(&mut app, size, &dir.join("06-watch.png"));

    // 7. The tag table.
    let mut app = EditorApp::new(rich_project());
    app.show_document(CentreTab::Tags);
    shoot(&mut app, size, &dir.join("07-symbols.png"));

    // 6. Window chrome: the shortcuts dialog.
    let mut app = EditorApp::new(rich_project());
    app.handle(softladder_ui::shortcuts::Action::ShortcutHelp);
    shoot(&mut app, size, &dir.join("08-shortcuts.png"));

    // 7. A narrow window, to see how the panels cope.
    let mut app = EditorApp::new(rich_project());
    shoot(
        &mut app,
        Vec2::new(1024.0, 700.0),
        &dir.join("09-small.png"),
    );

    // 8. A forced value: the warning banner and the force column.
    let mut app = EditorApp::new(rich_project());
    let var: softladder_core::VarRef = "%Q3".parse().expect("the example uses %Q3");
    softladder_ui::panels::watch::preload(
        vec![softladder_ui::WatchRow::new(var.clone())],
        vec![(var, true)],
    );
    app.show_document(CentreTab::Watch);
    shoot(&mut app, size, &dir.join("10-forced.png"));

    // 9. The dark theme, which every panel must draw with the same tokens.
    let mut app = EditorApp::new(rich_project());
    app.set_theme(softladder_ui::Theme::Dark);
    app.show_document(CentreTab::Tags);
    shoot(&mut app, size, &dir.join("11-dark.png"));

    // 12. The sequential document: an SFC section selected, with its page band,
    // the initial step, the AND divergence and the wiring between them. The
    // ribbon's Insert group shows the chart's own palette.
    let mut app = EditorApp::new(sfc_project());
    softladder_ui::sfc::open(&mut app, 1);
    shoot(&mut app, size, &dir.join("12-sfc-section.png"));

    // 13. Running: the active step filled, its elapsed time, and the transitions
    // that are ready to fire.
    let mut app = EditorApp::new(sfc_project());
    softladder_ui::sfc::open(&mut app, 1);
    app.handle(softladder_ui::shortcuts::Action::RunStop);
    for _ in 0..40 {
        app.single_scan();
    }
    // A scan reads its inputs from the operator's panel, so the values the shot
    // shows are written last, into the same store the document reads.
    for (text, value) in [("%I0", true), ("%I1", true), ("%I2", false), ("%I3", false)] {
        let var: softladder_core::VarRef = text.parse().expect("a valid variable");
        app.set_variable(&var, softladder_core::Value::Bit(value))
            .expect("the store accepts inputs");
    }
    shoot(&mut app, size, &dir.join("13-sfc-running.png"));

    // 14. A transition selected: the inspector shows its condition, the AND
    // badge and the steps it deactivates and activates.
    let mut app = EditorApp::new(sfc_project());
    softladder_ui::sfc::open(&mut app, 1);
    softladder_ui::sfc::focus(
        &mut app,
        0,
        Some(softladder_ui::sfc::Selection::Transition(0)),
    );
    shoot(&mut app, size, &dir.join("14-sfc-inspector.png"));

    // 14b. A step selected, with its number, its initial flag and its cell.
    let mut app = EditorApp::new(sfc_project());
    softladder_ui::sfc::open(&mut app, 1);
    softladder_ui::sfc::focus(&mut app, 0, Some(softladder_ui::sfc::Selection::Step(2)));
    shoot(&mut app, size, &dir.join("14b-sfc-step.png"));

    // 15. The chart's diagnostics: `SL-W002`, `SL-W001`, `SL-W011` and `SL-E011`
    // land in the Problems document with a page-and-element location.
    let mut app = EditorApp::new(broken_sfc_project());
    for code in ["SL-W002", "SL-W001", "SL-W011", "SL-E011"] {
        assert!(
            app.editor().problems().iter().any(|d| d.code == code),
            "the broken chart must report {code}: {:?}",
            app.editor().problems()
        );
    }
    app.show_document(CentreTab::Problems);
    shoot(&mut app, size, &dir.join("15-sfc-problems.png"));

    // 16. The dark theme on the sequential document, with the AND divergence
    // armed so the selected palette chip is visible too.
    let mut app = EditorApp::new(sfc_project());
    softladder_ui::sfc::open(&mut app, 1);
    softladder_ui::sfc::arm(
        &mut app,
        Some(softladder_ui::palette::SfcTool::AndDivergence),
    );
    app.set_theme(Theme::Dark);
    shoot(&mut app, size, &dir.join("16-sfc-dark.png"));
    softladder_ui::sfc::reset_view();
}
