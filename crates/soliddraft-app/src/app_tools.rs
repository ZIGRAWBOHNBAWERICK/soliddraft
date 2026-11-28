//! The registry's commands as tools over the app's state, for MCP clients and scripts.
//!
//! [`CommandTools`] is the part's command layer: the assistant's tools and the recipe
//! verbs. [`AppTools`] serves every command whose registry row has a tool: those, and
//! the file, export and drawing commands the ribbon runs, over the whole [`AppState`]
//! with its drawing. The MCP server in the app and on the command line, and
//! `soliddraft run`, serve it.
//!
//! A tool takes by name what the ribbon takes from the selection, and runs the same
//! typed commands and drawing helpers the app's action runs, so it is checked and undone
//! as the ribbon's is (the row's [`Undo`](crate::commands::Undo)). A command that needs
//! the window has no tool; its row says why.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use soliddraft_assistant::tools::schema::{any_of, array, boolean, integer, number, one_of, string};
use soliddraft_assistant::{Caller, ToolHost, ToolOutput, ToolSpec};
use soliddraft_document::DocKind;
use soliddraft_drawing::display::{annotation_name, field_value};
use soliddraft_drawing::export::Format as SheetFormat;
use soliddraft_drawing::measure::value_text;
use soliddraft_drawing::project::{self, view_label_in};
use soliddraft_drawing::{
    AnnotationId, AnnotationKind, AnnotationState, Boundary, Command as DrawingCommand, Drawing, FoldDirection,
    HiddenLines, PartIndex, SHEET, TangentEdges, ViewDisplay, ViewId, ViewKind, derived,
};
use soliddraft_exchange::Format as ExportFormat;
use soliddraft_kernel_api::{Point2, Vec3};
use soliddraft_model::pmi::inspection::{Applicability, InspectionPlan};

use crate::command_tools::{CommandTools, Surface, tool_specs};
use crate::document::{Command, Document};
use crate::drawing::document::DrawingDoc;
use crate::drawing::part::PartContext;
use crate::drawing::reference::LinkState;
use crate::drawing::{self, DrawingSession, NewDrawing};
use crate::state::AppState;

const NO_DRAWING: &str = "no drawing is open: make_drawing makes one of the part, and open opens a .dwdrw file";

/// A view, by its id or by a label only it has. Labels repeat: two sheets can each have
/// a Front view, so model_summary lists every view with its id.
fn view_schema(what: &str) -> Vec<Value> {
    vec![
        integer(&format!("{what}: the view's id, as model_summary lists it under drawing")),
        string(&format!(
            "{what}: a label only that view has, such as Front view or Section A-A, ignoring case; a label several \
             views share is refused, and their ids are listed"
        )),
    ]
}

fn view_param() -> (&'static str, Value) {
    ("view", any_of(view_schema("The view")))
}

fn path_param(what: &str) -> Vec<(&'static str, Value)> {
    vec![("path", string(what))]
}

/// The `open` tool's parameters.
pub fn open_params() -> Vec<(&'static str, Value)> {
    path_param("A part, drawing or assembly file path, ending in .dwprt, .dwdrw or .dwasm")
}

/// The `export_iges` tool's parameters.
pub fn iges_path_params() -> Vec<(&'static str, Value)> {
    path_param("The file to write, ending in .iges or .igs")
}

/// The `export_step` tool's parameters.
pub fn step_path_params() -> Vec<(&'static str, Value)> {
    path_param("The file to write, ending in .step or .stp")
}

fn mesh_path_params(description: &str) -> Vec<(&'static str, Value)> {
    let mut params = path_param(description);
    params.push(("accuracy_mm", soliddraft_assistant::tools::schema::optional(number("Explicit mesh accuracy in millimetres. A larger value is a coarser choice; does not change saved settings."))));
    params
}

/// The `export_stl` tool's parameters.
pub fn stl_path_params() -> Vec<(&'static str, Value)> {
    mesh_path_params("The file to write, ending in .stl")
}

/// The `export_3mf` tool's parameters.
pub fn three_mf_path_params() -> Vec<(&'static str, Value)> {
    mesh_path_params("The file to write, ending in .3mf")
}

/// The `export_obj` tool's parameters.
pub fn obj_path_params() -> Vec<(&'static str, Value)> {
    mesh_path_params("The file to write, ending in .obj")
}

/// The `drawing_export_pdf` tool's parameters.
pub fn pdf_path_params() -> Vec<(&'static str, Value)> {
    path_param("The file to write, ending in .pdf")
}

/// The `drawing_export_dxf` tool's parameters.
pub fn dxf_path_params() -> Vec<(&'static str, Value)> {
    path_param("The file to write, ending in .dxf")
}

/// The `drawing_export_svg` tool's parameters.
pub fn svg_path_params() -> Vec<(&'static str, Value)> {
    path_param("The file to write, ending in .svg")
}

/// The `drawing_save` tool's parameters.
pub fn drawing_path_params() -> Vec<(&'static str, Value)> {
    path_param("A drawing file path ending in .dwdrw")
}

/// The `make_drawing` tool's parameters.
pub fn make_drawing_params() -> Vec<(&'static str, Value)> {
    vec![(
        "template",
        string(
            "The sheet template's id: iso-a3 or iso-a4 (first angle), ansi-a or ansi-b (third angle), or one File \
             locations adds",
        ),
    )]
}

/// The parameters of a tool that takes one view.
pub fn view_params() -> Vec<(&'static str, Value)> {
    vec![view_param()]
}

/// The `drawing_hidden_lines` tool's parameters.
pub fn hidden_lines_params() -> Vec<(&'static str, Value)> {
    vec![view_param(), ("shown", boolean("true draws the hidden lines dashed, false removes them"))]
}

/// The `drawing_tangent_edges` tool's parameters.
pub fn tangent_edges_params() -> Vec<(&'static str, Value)> {
    vec![view_param(), ("tangent", one_of(&["removed", "phantom", "visible"], "How the view draws tangent edges"))]
}

/// The `drawing_crop` tool's parameters.
pub fn crop_params() -> Vec<(&'static str, Value)> {
    vec![
        view_param(),
        (
            "crop",
            any_of(vec![
                array(
                    number("mm"),
                    "The rectangle to keep, as two opposite corners [u1, v1, u2, v2] in sheet millimetres from the \
                     view's centre, each side at least 1 mm",
                ),
                json!({ "type": "null", "description": "Shows the whole view again" }),
            ]),
        ),
    ]
}

/// The `drawing_section` tool's parameters.
pub fn section_params() -> Vec<(&'static str, Value)> {
    vec![
        ("view", any_of(view_schema("The view to cut"))),
        (
            "through",
            array(number("mm"), "A model point [x, y, z] the cutting plane passes through, such as a hole's centre"),
        ),
        (
            "direction",
            one_of(&["horizontal", "vertical"], "The cutting line's direction across the view, on the sheet"),
        ),
        ("flip", boolean("true looks the other way along the cutting plane")),
    ]
}

/// The directions a projected view folds off its parent, as `drawing_projected` takes them.
const FOLDS: [&str; 8] = ["left", "right", "up", "down", "up_left", "up_right", "down_left", "down_right"];

/// The `drawing_projected` tool's parameters.
pub fn projected_params() -> Vec<(&'static str, Value)> {
    vec![
        ("view", any_of(view_schema("The view to fold from"))),
        (
            "direction",
            one_of(
                &FOLDS,
                "The side of the view the new one sits on; a diagonal side gives a pictorial view at a free place",
            ),
        ),
    ]
}

/// The `drawing_detail` tool's parameters.
pub fn detail_params() -> Vec<(&'static str, Value)> {
    vec![
        ("view", any_of(view_schema("The view to magnify a region of"))),
        (
            "center",
            array(number("mm"), "A model point [x, y, z] the detail circle is centred on, such as a hole's centre"),
        ),
        ("radius", number("The circle's radius in the view's millimetres on the sheet, above 0")),
        ("factor", number("How many times larger than its parent the detail is drawn, above 0, such as 2")),
    ]
}

/// The `drawing_auto_update` tool's parameters.
pub fn auto_update_params() -> Vec<(&'static str, Value)> {
    vec![("on", boolean("true rebuilds the drawing after every change of the open part"))]
}

/// The `drawing_auto_centre_marks` tool's parameters.
pub fn centre_marks_params() -> Vec<(&'static str, Value)> {
    vec![(
        "view",
        any_of(
            view_schema("The view whose circles get marks")
                .into_iter()
                .chain([json!({ "type": "null", "description": "Every view of the current sheet" })])
                .collect(),
        ),
    )]
}

/// The `drawing_delete` tool's parameters.
pub fn drawing_delete_params() -> Vec<(&'static str, Value)> {
    let or_null = |s: Value| any_of(vec![s, json!({ "type": "null" })]);
    vec![
        (
            "view",
            any_of(
                view_schema("The view to delete, with its annotations")
                    .into_iter()
                    .chain([json!({ "type": "null", "description": "Deletes the annotation instead" })])
                    .collect(),
            ),
        ),
        ("annotation", or_null(integer("The annotation's id, as model_summary lists it, or null to delete a view"))),
    ]
}

/// The registry's commands that have a tool, over the app's state.
pub struct AppTools<'a> {
    /// The part, its proposals and the drawing.
    pub state: &'a mut AppState,
}

impl AppTools<'_> {
    fn part(&mut self) -> CommandTools<'_> {
        let st = &mut *self.state;
        CommandTools {
            doc: &mut st.doc,
            book: &mut st.chat.book,
            surface: Surface::Mcp,
            drawing: st.drawing.session.as_mut().map(|s| &mut s.doc),
        }
    }

    /// What `model_summary` says: the part, and the open drawing.
    pub fn summary(&mut self) -> Value {
        let mut v = self.part().summary();
        let mut proof = self.state.doc.proof_state();
        if let Some(drag) = &self.state.drag_preview
            && let Some(request) = drag.current_request()
        {
            proof.requested_edit = Some(request);
            proof.displayed_edit = proof.displayed_edit.or_else(|| drag.displayed_request());
            proof.proved_edit = drag.proved_request();
            proof.transient = true;
            proof.pending = drag.pending();
            proof.failed = matches!(drag.shown(), crate::drag::Shown::Failed { .. });
        }
        v["edit_state"] = serde_json::json!(proof);
        if proof.transient {
            for key in ["volume_mm3", "area_mm2", "faces", "edges"] {
                v["body"][key] = Value::Null;
            }
        }
        if let Some(d) = drawing_json(self.state) {
            v["drawing"] = d;
        }
        if let Some(a) = crate::assembly::tools::summary(self.state) {
            v["assembly"] = a;
        }
        v
    }

    /// Runs the tools this host adds to the part's layer. `None` hands the call on.
    fn run(&mut self, name: &str, input: &Value) -> Option<Result<Value, String>> {
        if let Some(result) = crate::assembly::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::mates::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::interference::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::grid::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::subassembly::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::configurations::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::assembly_files::tools::run(self.state, name, input) {
            return Some(result);
        }
        if let Some(result) = crate::pmi::tools::run(&mut self.state.doc, name, input) {
            return Some(result);
        }
        let drawing_active = self.state.drawing.is_active();
        let assembly_active = self.state.assembly.is_active();
        Some(match name {
            "model_summary" => Ok(self.summary()),
            "undo" if assembly_active => crate::assembly::tools::step(self.state, true),
            "redo" if assembly_active => crate::assembly::tools::step(self.state, false),
            "undo" if drawing_active => self.step_drawing(true),
            "redo" if drawing_active => self.step_drawing(false),
            "open" => self.open(input)?,
            "import_scene" => crate::scene_import::run(self.state, input),
            "new_part" => self.new_part(),
            "export_iges" => self.export_part(ExportFormat::Iges, input),
            "export_step" => self.export_part(ExportFormat::Step, input),
            "export_stl" => self.export_part(ExportFormat::Stl, input),
            "export_3mf" => self.export_part(ExportFormat::ThreeMf, input),
            "export_obj" => self.export_part(ExportFormat::Obj, input),
            "export_stand_in" => self.export_part(ExportFormat::StandIn, input),
            "roll_to_end" => self.roll_to_end(),
            "make_drawing" => self.make_drawing(input),
            "manufacturing_package" => self.manufacturing_package(input),
            "show_drawing" => self.session().map(|_| ()).map(|()| {
                self.state.assembly.active = false;
                self.state.drawing.active = true;
                self.drawing_now()
            }),
            "drawing_show_part" => {
                self.state.assembly.active = false;
                self.state.drawing.active = false;
                Ok(json!({ "active": "part" }))
            }
            "drawing_align" => self.align(input),
            "drawing_break_alignment" => self.break_alignment(input),
            "drawing_hidden_lines" => self.hidden_lines(input),
            "drawing_tangent_edges" => self.tangent_edges(input),
            "drawing_crop" => self.crop(input),
            "drawing_section" => self.section(input),
            "drawing_projected" => self.projected(input),
            "drawing_detail" => self.detail(input),
            "drawing_rebuild" => self.rebuild(),
            "drawing_replace_model" => self.repoint(input, true),
            "drawing_repair_reference" => self.repoint(input, false),
            "drawing_auto_update" => self.auto_update(input),
            "drawing_model_items" => self.session().map(|_| ()).and_then(|()| {
                let said = drawing::import_model_items(self.state)?;
                Ok(self.drawing_said(said))
            }),
            "drawing_auto_centre_marks" => self.centre_marks(input),
            "drawing_add_sheet" => self.session().and_then(drawing::add_sheet).map(|name| self.drawing_said(name)),
            "drawing_export_pdf" => self.export_sheet(SheetFormat::Pdf, input),
            "drawing_export_dxf" => self.export_sheet(SheetFormat::Dxf, input),
            "drawing_export_svg" => self.export_sheet(SheetFormat::Svg, input),
            "drawing_save" => self.save_drawing(input),
            "drawing_close" => self.session().map(|_| ()).map(|()| {
                self.state.drawing.session = None;
                self.state.drawing.active = false;
                json!({ "closed": true })
            }),
            "drawing_delete" => self.delete(input),
            _ => return None,
        })
    }

    fn session(&mut self) -> Result<&mut DrawingSession, String> {
        self.state.drawing.session.as_mut().ok_or_else(|| NO_DRAWING.to_owned())
    }

    fn drawing_now(&self) -> Value {
        json!({ "drawing": drawing_json(self.state) })
    }

    fn drawing_said(&self, said: String) -> Value {
        json!({ "done": said, "drawing": drawing_json(self.state) })
    }

    /// Undo or redo in the drawing, while it is the active document.
    fn step_drawing(&mut self, undo: bool) -> Result<Value, String> {
        let s = self.session()?;
        if undo {
            let label = drawing::undo(s).ok_or("nothing to undo in the drawing")?;
            Ok(json!({ "undone": label, "document": "drawing", "undo": s.doc.undo_label() }))
        } else {
            let label = drawing::redo(s).ok_or("nothing to redo in the drawing")?;
            Ok(json!({ "redone": label, "document": "drawing" }))
        }
    }

    /// A drawing file opens here; a part file opens in the part's layer.
    fn open(&mut self, input: &Value) -> Option<Result<Value, String>> {
        let path = PathBuf::from(input["path"].as_str().unwrap_or_default());
        match DocKind::of_path(&path) {
            Some(DocKind::Part) => None,
            Some(DocKind::Assembly) => Some(crate::assembly::tools::open(self.state, path)),
            Some(DocKind::Drawing) => Some(self.open_drawing(path)),
            _ => Some(Err("the path must name a .dwprt part, .dwdrw drawing or .dwasm assembly file".into())),
        }
    }

    fn open_drawing(&mut self, path: PathBuf) -> Result<Value, String> {
        let doc = DrawingDoc::open(&path)?;
        self.state.assembly.active = false;
        let said = drawing::put_opened(self.state, doc);
        let missing = self.state.drawing.session.as_ref().is_some_and(|s| s.link_state() == LinkState::Missing);
        let part = if missing {
            "not found: drawing_repair_reference points the drawing at the part file".to_owned()
        } else {
            self.open_model()?
        };
        Ok(json!({ "opened": path, "done": said, "part": part, "drawing": drawing_json(self.state) }))
    }

    /// Opens the drawing's part in place of the open part, as the app does after opening
    /// a drawing or repairing its reference. Says which part is open.
    fn open_model(&mut self) -> Result<String, String> {
        if let Some(file) = drawing::model_to_open(self.state)?
            && DocKind::of_path(&file) == Some(DocKind::Assembly)
        {
            crate::assembly::tools::open(self.state, file)?;
            self.state.assembly.active = false;
            self.state.drawing.active = true;
            return Ok(self.state.assembly.session.as_ref().map(|a| a.doc.assembly().name.clone()).unwrap_or_default());
        }
        if let Some(file) = drawing::model_to_open(self.state)? {
            let caller = Caller::Mcp { client: "drawing".into() };
            let out = self.part().call("open", json!({ "path": file }), &caller);
            if out.is_error {
                return Err(format!("the drawing's part did not open: {}", out.content));
            }
        }
        Ok(self.state.doc.name.clone())
    }

    fn new_part(&mut self) -> Result<Value, String> {
        let st = &mut *self.state;
        let kernel = st.doc.kernel().clone();
        st.editor = None;
        st.sketch = None;
        st.chat.preview = None;
        st.drag_preview = None;
        st.assembly.active = false;
        st.drawing.active = false;
        st.doc = Document::new("Part1", kernel);
        Ok(self.summary())
    }

    /// Writes the part's body as `format`, as File › Export does.
    fn export_part(&mut self, format: ExportFormat, input: &Value) -> Result<Value, String> {
        let path = PathBuf::from(input["path"].as_str().unwrap_or_default());
        if !format.suits(&path) {
            return Err(format!("the path must end in .{} for {}", format.extension(), format.name()));
        }
        let accuracy = input.get("accuracy_mm").and_then(Value::as_f64);
        let out = self.state.export_part_at(format, accuracy).map_err(|error| {
            if crate::print_check::is_mesh(format) && crate::print_check::can_coarsen(&error) {
                format!("{error}. Try an explicit coarser accuracy_mm. A larger value allows more deviation; no file was written.")
            } else { error }
        })?;
        out.write_with(&path, crate::platform::write_file)?;
        let mut result = json!({ "exported": path, "report": out.report.summary(), "evidence": out.report });
        if let Some(pmi) = &out.pmi {
            result["manufacturing_information"] = json!(pmi);
        }
        Ok(result)
    }

    fn roll_to_end(&mut self) -> Result<Value, String> {
        if self.state.doc.rollback.is_none() {
            return Err("the rollback bar is already at the end".into());
        }
        self.state.doc.apply(Command::Rollback(None))?;
        Ok(self.summary())
    }

    /// One action for the shop: the part's STEP file with its manufacturing information,
    /// a drawing made from that information as PDF and DXF beside it and, unless
    /// `inspection` is false, its inspection plan as CSV and an AS9102 first article
    /// workbook. The answer says what each file holds and lists, by name, every item the
    /// drawing could not show, every text that lies over something, anything on the
    /// drawing that the file does not state, and every characteristic that cannot be
    /// inspected as it stands.
    pub(crate) fn manufacturing_package(&mut self, input: &Value) -> Result<Value, String> {
        let step = package_path(input)?;
        let template = input["template"].as_str().unwrap_or("iso-a3");
        let inspection = package_inspection(input)?;
        let mut new = self.drawing_request(&json!({ "template": template }))?;
        // Front, top and side; the section through the tightest hole comes with the
        // manufacturing information.
        for (view, on) in &mut new.views {
            if *view == soliddraft_kernel_api::drawing::StandardView::Isometric {
                *on = false;
            }
        }
        // Everything is made before anything is written, so a refusal leaves no file.
        let exported = self.state.export_part_at(ExportFormat::Step, None)?;
        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        exported.write_with(&step, |file, bytes| {
            files.push((file.to_owned(), bytes.to_vec()));
            Ok(())
        })?;
        let on_body = self.state.doc.model().pmi_on_body();
        let written: std::collections::BTreeSet<String> =
            on_body.pmi.items.iter().take(on_body.items).map(|item| item.id.clone()).collect();
        let inspection = match inspection {
            Some(stated) => {
                let plan = InspectionPlan::from_document(self.state.doc.model(), stated.revision.as_deref());
                let (csv, workbook) = (step.with_extension("csv"), step.with_extension("xlsx"));
                let mut table = Vec::new();
                plan.write_csv(&mut table).map_err(|e| format!("the inspection CSV: {e}"))?;
                let forms =
                    plan.as9102_xlsx(stated.part_number.as_deref()).map_err(|e| format!("the AS9102 workbook: {e}"))?;
                files.push((csv.clone(), table));
                files.push((workbook.clone(), forms));
                inspection_report(&plan, &stated, &csv, &workbook)
            }
            None => Value::Null,
        };
        let session = drawing::create_drawing(self.state, &new)?;
        let requirements = session.requirements.clone();
        self.install_drawing(session);
        let (pdf, dxf) = (step.with_extension("pdf"), step.with_extension("dxf"));
        let prepared = drawing::prepare_sheets(self.session()?, false)?;
        let accuracy = prepared.package_accuracy();
        for (format, path) in [(SheetFormat::Pdf, &pdf), (SheetFormat::Dxf, &dxf)] {
            files.push((path.clone(), prepared.bytes(format)?));
        }
        // On the desktop each is a file beside the STEP file; in the browser they arrive
        // as one download.
        crate::platform::write_export(&step, files)?;
        let s = self.session()?;
        let sheet = s.sheet;
        let drawing = &s.doc.drawing;
        let overlaps: Vec<Value> = soliddraft_drawing::pmi_items::overlaps(drawing, sheet)
            .into_iter()
            .map(|(what, over)| json!({ "annotation": what, "over": over }))
            .collect();
        let shown = soliddraft_drawing::pmi_items::shown(drawing);
        Ok(json!({
            "step": step,
            "pdf": pdf,
            "dxf": dxf,
            "manufacturing_information": exported.pmi,
            "drawing": {
                "views": drawing.sheets[sheet].views.iter().filter(|v| v.is_printed()).count(),
                "accuracy": accuracy,
                "items": requirements.placed,
                "shown": shown,
                "not_shown": requirements.unplaced,
                "overlaps": overlaps,
                "not_in_file": soliddraft_drawing::pmi_items::not_from_part(drawing, &written),
            },
            "inspection": inspection,
        }))
    }

    /// Makes a drawing as the New drawing PropertyManager does with its defaults.
    fn make_drawing(&mut self, input: &Value) -> Result<Value, String> {
        let new = self.drawing_request(input)?;
        let session = drawing::create_drawing(self.state, &new)?;
        Ok(self.install_drawing(session))
    }

    pub(crate) fn drawing_request(&self, input: &Value) -> Result<NewDrawing, String> {
        let id = input["template"].as_str().unwrap_or_default().trim();
        check_drawing(&self.state.doc, id)?;
        let mut new = NewDrawing::new();
        new.set_template(id);
        Ok(new)
    }

    pub(crate) fn install_drawing(&mut self, mut session: DrawingSession) -> Value {
        session.seen_version = self.state.doc.mesh_version;
        session.doc.log_mark = self.state.doc.command_log().len();
        self.state.assembly.active = false;
        self.state.drawing.session = Some(session);
        self.state.drawing.new = None;
        self.state.drawing.active = true;
        self.state.tool_document = Some(crate::app::Tab::Drawing);
        self.drawing_now()
    }

    /// The view `input` names, with the sheet's name for it.
    fn view(&mut self, input: &Value) -> Result<(ViewId, String), String> {
        let s = self.session()?;
        find_view(&s.doc.drawing, &input["view"])
    }

    /// Runs a drawing command as one undo step and answers with the drawing.
    fn command(&mut self, cmd: DrawingCommand) -> Result<Value, String> {
        drawing::run_command(self.state, cmd)?;
        Ok(self.drawing_now())
    }

    fn align(&mut self, input: &Value) -> Result<Value, String> {
        let (view, name) = self.view(input)?;
        let v = self.session()?.doc.drawing.view(view).ok_or(NO_DRAWING)?;
        if v.parent().is_none() {
            return Err(format!("`{name}` has no parent view to align with"));
        }
        if v.alignment.is_some() {
            return Err(format!("`{name}` is aligned with its parent already"));
        }
        self.command(DrawingCommand::Align { view })
    }

    fn break_alignment(&mut self, input: &Value) -> Result<Value, String> {
        let (view, name) = self.view(input)?;
        if self.session()?.doc.drawing.view(view).is_none_or(|v| v.alignment.is_none()) {
            return Err(format!("`{name}` is not aligned with a parent"));
        }
        self.command(DrawingCommand::BreakAlignment { view })
    }

    fn display(&mut self, view: ViewId, change: impl FnOnce(&mut ViewDisplay)) -> Result<Value, String> {
        let mut display = self.session()?.doc.drawing.view(view).ok_or(NO_DRAWING)?.display;
        change(&mut display);
        self.command(DrawingCommand::SetViewDisplay { view, display })
    }

    fn hidden_lines(&mut self, input: &Value) -> Result<Value, String> {
        let (view, _) = self.view(input)?;
        let hidden =
            if input["shown"].as_bool().unwrap_or(false) { HiddenLines::Visible } else { HiddenLines::Removed };
        self.display(view, |d| d.hidden = hidden)
    }

    fn tangent_edges(&mut self, input: &Value) -> Result<Value, String> {
        let (view, _) = self.view(input)?;
        let tangent = match input["tangent"].as_str().unwrap_or_default() {
            "removed" => TangentEdges::Removed,
            "phantom" => TangentEdges::Phantom,
            "visible" => TangentEdges::Visible,
            other => return Err(format!("tangent edges are removed, phantom or visible, not `{other}`")),
        };
        self.display(view, |d| d.tangent = tangent)
    }

    fn crop(&mut self, input: &Value) -> Result<Value, String> {
        let (view, name) = self.view(input)?;
        let cropped = self.session()?.doc.drawing.view(view).is_some_and(|v| v.crop.is_some());
        let crop = match &input["crop"] {
            Value::Null if !cropped => return Err(format!("`{name}` is not cropped")),
            Value::Null => None,
            corners => {
                let n: Vec<f64> = corners.as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
                let [u1, v1, u2, v2] = n[..] else {
                    return Err("a crop is four numbers: two opposite corners [u1, v1, u2, v2]".into());
                };
                if n.iter().any(|x| !x.is_finite()) || (u1 - u2).abs() < 1.0 || (v1 - v2).abs() < 1.0 {
                    return Err("each side of the crop must be at least 1 mm".into());
                }
                Some(Boundary::rectangle(Point2::new(u1, v1), Point2::new(u2, v2)))
            }
        };
        self.command(DrawingCommand::SetViewCrop { view, crop })
    }

    /// Section: cuts a view along a horizontal or vertical line through a model point
    /// and places the section in line with it, as the canvas tool does after its two
    /// clicks. The section is drawn from the open part, which must be the drawing's.
    fn section(&mut self, input: &Value) -> Result<Value, String> {
        let (parent, name) = self.view(input)?;
        let n: Vec<f64> = input["through"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
        let [x, y, z] = n[..] else {
            return Err("through is a model point: three numbers [x, y, z]".into());
        };
        if !n.iter().all(|v| v.is_finite()) {
            return Err("through must be three finite numbers".into());
        }
        let horizontal = match input["direction"].as_str().unwrap_or_default() {
            "horizontal" => true,
            "vertical" => false,
            other => return Err(format!("the direction is horizontal or vertical, not `{other}`")),
        };
        let flip = input["flip"].as_bool().unwrap_or(false);
        if crate::assembly_files::drawing::drawing_is_of_assembly(self.state) {
            let placed =
                crate::assembly_files::drawing::section(self.state, parent, Vec3::new(x, y, z), horizontal, flip)?;
            return Ok(json!({ "placed": placed.0, "id": placed.1, "drawing": drawing_json(self.state) }));
        }
        if !drawing::part_is_model(self.state) {
            return Err("The open part is not the one this drawing shows".into());
        }
        let (sheet, view, id) = {
            let st = &mut *self.state;
            let ctx = PartContext::new(&st.doc).ok_or("The part has no body; extrude something first")?;
            let session = st.drawing.session.as_mut().ok_or_else(|| NO_DRAWING.to_owned())?;
            if matches!(session.doc.drawing.view(parent).map(|v| &v.kind), Some(ViewKind::Detail { .. })) {
                return Err(format!("`{name}` is a detail view and cannot be sectioned; cut its parent"));
            }
            let sheet = project::sheet_of(&session.doc.drawing, parent).ok_or(NO_DRAWING)?;
            let part = ctx.part();
            let index = PartIndex::build(&part).map_err(|e| e.to_string())?;
            let drawing = &session.doc.drawing;
            let line = derived::cutting_line(drawing, &part, &index, parent, Vec3::new(x, y, z), horizontal)
                .map_err(|e| format!("Section: {e}"))?;
            let (view, vctx) = derived::new_section(drawing, &part, &index, sheet, parent, line, flip, None)
                .map_err(|e| format!("Section: {e}"))?;
            let id = view.id;
            session.doc.remember(id, &vctx.projected);
            (sheet, view, id)
        };
        drawing::run_command(self.state, DrawingCommand::PlaceView { sheet, view })?;
        let drawing = &self.session()?.doc.drawing;
        let placed =
            drawing.sheet_of(id).and_then(|s| s.view(id).map(|v| view_label_in(drawing, s, v))).unwrap_or_default();
        Ok(json!({ "placed": placed, "id": id.0, "drawing": drawing_json(self.state) }))
    }

    /// Projected: folds a view off a view on one of eight sides, as the canvas tool does
    /// after its click. A part drawing's view is drawn from the open part, which must be
    /// the drawing's; an assembly drawing's from the open assembly.
    fn projected(&mut self, input: &Value) -> Result<Value, String> {
        let (parent, _) = self.view(input)?;
        let said = input["direction"].as_str().unwrap_or_default();
        let direction: FoldDirection = serde_json::from_value(json!(said))
            .map_err(|_| format!("the direction is one of {}, not `{said}`", FOLDS.join(", ")))?;
        if crate::assembly_files::drawing::drawing_is_of_assembly(self.state) {
            let placed = crate::assembly_files::drawing::projected(self.state, parent, direction)?;
            return Ok(json!({ "placed": placed.0, "id": placed.1, "drawing": drawing_json(self.state) }));
        }
        self.place_part_view(parent, "Projected", |drawing, part, index, sheet| {
            derived::new_projected(drawing, part, index, sheet, parent, direction)
        })
    }

    /// Detail: magnifies the region of a view inside a circle round a model point, as the
    /// canvas tool does after its clicks, placed at a free place near the view.
    fn detail(&mut self, input: &Value) -> Result<Value, String> {
        let (parent, _) = self.view(input)?;
        let n: Vec<f64> = input["center"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
        let [x, y, z] = n[..] else {
            return Err("center is a model point: three numbers [x, y, z]".into());
        };
        let radius = input["radius"].as_f64().unwrap_or(f64::NAN);
        let factor = input["factor"].as_f64().unwrap_or(f64::NAN);
        let positive = |v: f64| v > 0.0 && v.is_finite();
        if !n.iter().all(|v| v.is_finite()) || !positive(radius) || !positive(factor) {
            return Err("center must be three finite numbers, and radius and factor finite numbers above 0".into());
        }
        let center = Vec3::new(x, y, z);
        if crate::assembly_files::drawing::drawing_is_of_assembly(self.state) {
            let placed = crate::assembly_files::drawing::detail(self.state, parent, center, radius, factor)?;
            return Ok(json!({ "placed": placed.0, "id": placed.1, "drawing": drawing_json(self.state) }));
        }
        self.place_part_view(parent, "Detail", |drawing, part, index, sheet| {
            let at = derived::context(drawing, part, index, parent)?.xf.point(center);
            derived::new_detail(drawing, part, index, sheet, parent, at, radius, Vec::new(), factor, None)
        })
    }

    /// Places a view `make` derives from the part drawing's view `parent` as one undo step,
    /// drawn from the open part, which must be the drawing's. Answers with its label.
    fn place_part_view(
        &mut self,
        parent: ViewId,
        what: &str,
        make: impl FnOnce(
            &Drawing,
            &soliddraft_drawing::Part<'_>,
            &PartIndex<'_>,
            usize,
        )
            -> Result<(soliddraft_drawing::View, project::ViewContext), soliddraft_drawing::DrawingError>,
    ) -> Result<Value, String> {
        if !drawing::part_is_model(self.state) {
            return Err("The open part is not the one this drawing shows".into());
        }
        let (sheet, view, id) = {
            let st = &mut *self.state;
            let ctx = PartContext::new(&st.doc).ok_or("The part has no body; extrude something first")?;
            let session = st.drawing.session.as_mut().ok_or_else(|| NO_DRAWING.to_owned())?;
            let sheet = project::sheet_of(&session.doc.drawing, parent).ok_or(NO_DRAWING)?;
            let part = ctx.part();
            let index = PartIndex::build(&part).map_err(|e| e.to_string())?;
            let (view, vctx) = make(&session.doc.drawing, &part, &index, sheet).map_err(|e| format!("{what}: {e}"))?;
            let id = view.id;
            session.doc.remember(id, &vctx.projected);
            (sheet, view, id)
        };
        drawing::run_command(self.state, DrawingCommand::PlaceView { sheet, view })?;
        let drawing = &self.session()?.doc.drawing;
        let placed =
            drawing.sheet_of(id).and_then(|s| s.view(id).map(|v| view_label_in(drawing, s, v))).unwrap_or_default();
        Ok(json!({ "placed": placed, "id": id.0, "drawing": drawing_json(self.state) }))
    }

    /// Rebuild: opens the drawing's part when it is not the open part, then brings the
    /// views to its revision.
    fn rebuild(&mut self) -> Result<Value, String> {
        self.session()?;
        if crate::assembly_files::drawing::drawing_is_of_assembly(self.state) {
            let wanted = self.session()?.doc.drawing.model.id;
            if self.state.assembly.session.as_ref().map(|a| a.doc.id()) != wanted {
                let file = drawing::model_to_open(self.state)?
                    .ok_or("the drawing's assembly was not found; drawing_repair_reference points it at the file")?;
                crate::assembly::tools::open(self.state, file)?;
            }
            let said = drawing::update_views(self.state)?;
            return Ok(self.drawing_said(said));
        }
        if !drawing::part_is_model(self.state) {
            if drawing::model_to_open(self.state)?.is_none() {
                return Err(
                    "the drawing's part was not found; drawing_repair_reference points the drawing at it".into()
                );
            }
            self.open_model()?;
        }
        let said = drawing::update_views(self.state)?;
        Ok(self.drawing_said(said))
    }

    fn repoint(&mut self, input: &Value, replace: bool) -> Result<Value, String> {
        self.session()?;
        let path = PathBuf::from(input["path"].as_str().unwrap_or_default());
        if DocKind::of_path(&path) != Some(DocKind::Part) {
            return Err("the path must name a .dwprt part file".into());
        }
        let said = drawing::repoint_model(self.state, &path, replace)?;
        let part = self.open_model()?;
        Ok(json!({ "done": said, "part": part, "drawing": drawing_json(self.state) }))
    }

    fn auto_update(&mut self, input: &Value) -> Result<Value, String> {
        let on = input["on"].as_bool().unwrap_or(false);
        if drawing::set_automatic_update(self.session()?, on)? {
            return self.rebuild();
        }
        Ok(self.drawing_now())
    }

    fn centre_marks(&mut self, input: &Value) -> Result<Value, String> {
        let views = match input["view"] {
            Value::Null => self.session()?.sheet().views.iter().map(|v| v.id).collect(),
            _ => vec![self.view(input)?.0],
        };
        let s = self.state.drawing.session.as_mut().ok_or(NO_DRAWING)?;
        let added = drawing::add_centre_marks(s, &self.state.doc, &views);
        Ok(self.drawing_said(format!("{added} centre mark{} added", if added == 1 { "" } else { "s" })))
    }

    fn export_sheet(&mut self, format: SheetFormat, input: &Value) -> Result<Value, String> {
        let path = PathBuf::from(input["path"].as_str().unwrap_or_default());
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(format.extension())) {
            return Err(format!("the path must end in .{}", format.extension()));
        }
        let s = self.state.drawing.session.as_mut().ok_or_else(|| NO_DRAWING.to_owned())?;
        let written = drawing::write_sheet(s, format, &path)?;
        Ok(json!({
            "exported": path,
            "views": written.views,
            "method": if written.views == 0 { "none" } else { "catallax" },
            "approximate": written.accuracy.approximate > 0,
            "revision": if written.view_revisions.len() == 1 {
                written.view_revisions.first()
            } else {
                None
            },
            "pinned_revision": written.pinned_revision.short(),
            "view_revisions": written.view_revisions,
            "accuracy": written.accuracy,
        }))
    }

    fn save_drawing(&mut self, input: &Value) -> Result<Value, String> {
        let path = PathBuf::from(input["path"].as_str().unwrap_or_default());
        if DocKind::of_path(&path) != Some(DocKind::Drawing) {
            return Err("the path must name a .dwdrw drawing file".into());
        }
        let backend = self.state.doc.kernel().backend();
        let kernel = format!("{} {}", backend.name, backend.version);
        let (path, warning) = drawing::save_session(self.session()?, &path, &kernel)?;
        Ok(json!({ "saved": path, "warning": warning }))
    }

    fn delete(&mut self, input: &Value) -> Result<Value, String> {
        match (input["view"].is_null(), input["annotation"].as_u64()) {
            (false, None) if input["annotation"].is_null() => {
                let (view, _) = self.view(input)?;
                self.command(DrawingCommand::DeleteView { view })
            }
            (true, Some(n)) => {
                let annotation = u32::try_from(n).map(AnnotationId).map_err(|_| format!("no annotation {n}"))?;
                if self.session()?.doc.drawing.annotation(annotation).is_none() {
                    return Err(format!("no annotation {n}; model_summary lists them under drawing"));
                }
                self.command(DrawingCommand::DeleteAnnotation { annotation })
            }
            _ => Err("name either a view or an annotation, and set the other to null".into()),
        }
    }
}

impl ToolHost for AppTools<'_> {
    fn tools(&self) -> Vec<ToolSpec> {
        tool_specs(Surface::Mcp)
    }

    fn call(&mut self, name: &str, input: Value, caller: &Caller) -> ToolOutput {
        let opening_part = name == "open"
            && DocKind::of_path(std::path::Path::new(input["path"].as_str().unwrap_or_default()))
                == Some(DocKind::Part);
        let out = match name {
            // A copy in another file is a save in that file, as in the app.
            "save_as" => self.part().call("save", input, caller),
            _ => match self.run(name, &input) {
                Some(Ok(v)) => ToolOutput::json(&v),
                Some(Err(e)) => ToolOutput::error(e),
                None => self.part().call(name, input, caller),
            },
        };
        if out.is_error && out.content.contains("(SD-DISPLAY-") {
            self.state.doc.record_display_refusal(out.content.clone());
        }
        if opening_part && !out.is_error {
            self.state.assembly.active = false;
            self.state.drawing.active = false;
        }
        if !out.is_error
            && matches!(
                name,
                "make_drawing"
                    | "manufacturing_package"
                    | "show_drawing"
                    | "make_assembly"
                    | "show_assembly"
                    | "drawing_show_part"
                    | "assembly_show_part"
                    | "new_part"
                    | "open"
                    | "import_scene"
            )
        {
            self.state.tool_document = Some(if self.state.assembly.is_active() {
                crate::app::Tab::Assembly
            } else if self.state.drawing.is_active() {
                crate::app::Tab::Drawing
            } else {
                crate::app::Tab::Viewport
            });
        }
        // What the window's frames do after a change: the checked mesh replaces a GPU
        // preview, and the drawing follows its part.
        self.state.doc.finish_display();
        drawing::follow_part(self.state);
        out
    }
}

/// The STEP file a manufacturing package writes: its input's `path`, exactly as given.
/// The package card shows this path and Apply writes to it; the assistant's path is
/// trimmed once, before its card is made (`pmi::assistant::normalized`).
pub(crate) fn package_path(input: &Value) -> Result<PathBuf, String> {
    let step = PathBuf::from(input["path"].as_str().unwrap_or_default());
    if !ExportFormat::Step.suits(&step) {
        return Err("the path must end in .step or .stp".into());
    }
    Ok(step)
}

/// What the user stated for a package's inspection files: `None` when the input turns
/// them off. A part number and a revision are only ever what the input says.
pub(crate) fn package_inspection(input: &Value) -> Result<Option<InspectionStated>, String> {
    let stated = |key: &str| match &input[key] {
        Value::Null => Ok(None),
        Value::String(text) if text.trim().is_empty() => Ok(None),
        Value::String(text) => Ok(Some(text.trim().to_owned())),
        _ => Err(format!("`{key}` is text")),
    };
    let wanted = match &input["inspection"] {
        Value::Null => true,
        Value::Bool(wanted) => *wanted,
        _ => return Err("`inspection` is true or false".into()),
    };
    let (part_number, revision) = (stated("part_number")?, stated("revision")?);
    if !wanted && (part_number.is_some() || revision.is_some()) {
        return Err("a part number or revision goes on the inspection files; leave `inspection` on".into());
    }
    Ok(wanted.then_some(InspectionStated { part_number, revision }))
}

/// The part number and revision a user stated for the inspection files.
pub(crate) struct InspectionStated {
    pub(crate) part_number: Option<String>,
    pub(crate) revision: Option<String>,
}

/// What the inspection files hold: how many characteristics, how many to measure, and
/// each one that cannot be measured as it stands, by ID with the reason.
fn inspection_report(plan: &InspectionPlan, stated: &InspectionStated, csv: &Path, workbook: &Path) -> Value {
    let (mut inspect, mut reference, mut unresolved) = (0, 0, Vec::new());
    for group in &plan.groups {
        for row in &group.rows {
            match row.applicability(group) {
                Applicability::Inspect => inspect += 1,
                Applicability::Reference => reference += 1,
                Applicability::Unresolved => unresolved.push(json!({
                    "id": row.id,
                    "problem": group.problem.as_deref().unwrap_or("The source PMI has no nominal for this tolerance."),
                })),
            }
        }
    }
    json!({
        "csv": csv,
        "workbook": workbook,
        "part_number": stated.part_number,
        "revision": stated.revision,
        "characteristics": inspect + reference + unresolved.len(),
        "inspect": inspect,
        "reference": reference,
        "unresolved": unresolved,
    })
}

/// Whether a drawing of `doc` can be made now on the template `id`.
pub(crate) fn check_drawing(doc: &Document, id: &str) -> Result<(), String> {
    let id = id.trim();
    let templates = &soliddraft_drawing::library::library().templates;
    if !templates.iter().any(|t| t.id == id) {
        let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        return Err(format!("no template `{id}` (there are: {})", ids.join(", ")));
    }
    if doc.display_body().is_none() {
        return Err("Make drawing needs a body; extrude something first".into());
    }
    if !doc.proof_state().current_proved_model() {
        return Err("Wait for the part edit to finish before making a drawing".into());
    }
    Ok(())
}

/// The view `wanted` means, on any sheet: its id, or a label no other view has,
/// ignoring case. Returns it with a name for messages. A label several views share is
/// refused with their ids, so a command never lands on one of them by chance.
fn find_view(drawing: &Drawing, wanted: &Value) -> Result<(ViewId, String), String> {
    let all = || drawing.sheets.iter().flat_map(|sheet| sheet.views.iter().map(move |v| (sheet, v)));
    let listed = || {
        let views: Vec<String> = all()
            .map(|(sheet, v)| format!("{} (id {}, {})", view_label_in(drawing, sheet, v), v.id.0, sheet.name))
            .collect();
        if views.is_empty() { "no views".to_owned() } else { views.join(", ") }
    };
    if let Some(n) = wanted.as_u64() {
        return all()
            .find(|(_, v)| u64::from(v.id.0) == n)
            .map(|(sheet, v)| (v.id, view_label_in(drawing, sheet, v)))
            .ok_or_else(|| format!("no view has id {n} (the drawing has: {})", listed()));
    }
    let name = wanted.as_str().unwrap_or_default().trim();
    let found: Vec<_> =
        all().filter(|(sheet, v)| view_label_in(drawing, sheet, v).eq_ignore_ascii_case(name)).collect();
    match found[..] {
        [(sheet, v)] => Ok((v.id, view_label_in(drawing, sheet, v))),
        [] => Err(format!("no view named `{name}` (the drawing has: {})", listed())),
        _ => {
            let ids: Vec<String> = found.iter().map(|(sheet, v)| format!("{} ({})", v.id.0, sheet.name)).collect();
            Err(format!("`{name}` names {} views, ids {}; give the id of the one meant", found.len(), ids.join(", ")))
        }
    }
}

/// The serde tag of a view kind or annotation body: "section", "dimension".
fn kind_name<T: serde::Serialize>(kind: &T) -> Value {
    serde_json::to_value(kind).ok().and_then(|v| v.get("type").cloned()).unwrap_or(Value::Null)
}

/// The open drawing as agents read it, with named views and annotations. The lists of
/// changed and dangling annotations come from the same checks as the Needs attention panel.
fn drawing_json(state: &AppState) -> Option<Value> {
    let s = state.drawing.session.as_ref()?;
    let d = &s.doc.drawing;
    let refusals: std::collections::BTreeMap<_, _> =
        s.doc.refused_views().map(|(_, id, name, cause)| (id, (name, cause))).collect();
    let (mut changed, mut dangling) = (Vec::new(), Vec::new());
    let sheets: Vec<Value> = d
        .sheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let style = crate::drawing::document::DrawingDoc::style(sheet);
            let name_of = |id: ViewId| d.sheet_of(id).and_then(|s| s.view(id).map(|v| view_label_in(d, s, v)));
            let views: Vec<Value> = sheet
                .views
                .iter()
                .map(|v| {
                    json!({
                        "id": v.id.0,
                        "name": view_label_in(d, sheet, v),
                        "kind": kind_name(&v.kind),
                        "parent": v.parent().and_then(name_of),
                        "parent_id": v.parent().map(|p| p.0),
                        "aligned": v.alignment.is_some(),
                        "hidden_lines": v.display.hidden == HiddenLines::Visible,
                        "tangent_edges": match v.display.tangent {
                            TangentEdges::Removed => "removed",
                            TangentEdges::Phantom => "phantom",
                            TangentEdges::Visible => "visible",
                        },
                        "cropped": v.crop.is_some(),
                        "excluded_from_update": v.excluded_from_update,
                        "up_to_date": s.doc.view_is_up_to_date(v),
                        "preview": v.geometry.unproved,
                        "curves": v.geometry.curves.len(),
                        "accuracy": v.geometry.accuracy(),
                    })
                })
                .collect();
            let annotations: Vec<Value> = sheet
                .annotations
                .iter()
                .map(|a| {
                    let view = if a.view == SHEET { Some("sheet".to_owned()) } else { name_of(a.view) };
                    let view_id = (a.view != SHEET).then_some(a.view.0);
                    let name = annotation_name(a, style);
                    let value = match &a.body {
                        AnnotationKind::Dimension(dim) => Some(value_text(dim)),
                        AnnotationKind::DimensionSet(set) => Some(set.values_text()),
                        AnnotationKind::HoleCallout { text } => Some(text.clone()),
                        _ => None,
                    };
                    let references: Vec<&str> = a.anchors.iter().map(|anchor| anchor.entity.role.as_str()).collect();
                    let (state, previous, cause) = match &a.state {
                        AnnotationState::Ok => ("ok", None, None),
                        AnnotationState::Changed { previous } => {
                            changed.push(json!({ "id": a.id.0, "name": name, "previous": previous, "now": value }));
                            ("changed", Some(previous.clone()), None)
                        }
                        AnnotationState::Dangling { cause, .. } => {
                            dangling.push(json!({ "id": a.id.0, "name": name, "cause": cause }));
                            ("dangling", None, Some(cause.clone()))
                        }
                    };
                    json!({
                        "id": a.id.0,
                        "name": name,
                        "kind": kind_name(&a.body),
                        "view": view,
                        "view_id": view_id,
                        "value": value,
                        "references": references,
                        "state": state,
                        "previous": previous,
                        "cause": cause,
                    })
                })
                .collect();
            let title_block: serde_json::Map<String, Value> =
                soliddraft_drawing::library::template(sheet.template.as_str())
                    .title_block
                    .fields
                    .iter()
                    .map(|field| (field.key.clone(), field_value(d, index, field).into()))
                    .collect();
            json!({
                "name": sheet.name, "views": views, "annotations": annotations,
                "title_block": title_block, "title_bindings": sheet.title_bindings,
            })
        })
        .collect();
    let part = match s.link_state() {
        LinkState::Current => "current",
        LinkState::Newer { .. } => "newer: drawing_rebuild brings the views to it",
        LinkState::Missing => "missing: drawing_repair_reference points the drawing at it",
        LinkState::Unverified => "not a SolidDraft document",
        LinkState::Unknown => "not resolved",
    };
    Some(json!({
        "title": d.meta.title,
        "path": s.doc.path,
        "active": state.drawing.is_active(),
        "sheet": s.sheet().name,
        "sheets": sheets,
        "up_to_date": s.is_up_to_date(),
        "pinned_revision": d.model.pinned.short(),
        "source": {
            "revision": d.model.pinned.as_str(),
            "configuration": d.model.configuration,
            "properties": d.source_properties,
        },
        "part_revision": s.doc.seen_digest.short(),
        "views_match_pin": d.views_match_pin(),
        "changed": changed,
        "dangling": dangling,
        "refused_views": refusals.iter().map(|(id, (name, cause))| {
            json!({ "id": id.0, "name": name, "cause": cause })
        }).collect::<Vec<_>>(),
        "part": part,
        "automatic_update": d.settings.automatic_update,
        "undo": s.doc.undo_label(),
    }))
}
