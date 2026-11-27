//! SolidDraft: a parametric CAD application in Rust. Phase 3: the assistant.
//!
//! The crate is a library so the integration tests can drive the real app
//! through `egui_kittest`; `main.rs` is the thin binary.

// The interface modules are public only so the integration tests can reach them.
#![allow(missing_docs)]

pub mod app;
pub mod app_tools;
pub mod assembly;
pub mod assembly_files;
pub mod bottom_popovers;
pub mod chat;
pub mod command_line;
#[macro_use]
pub mod command_tools;
pub mod commands;
pub mod config_tabs;
pub mod configurations;
pub mod context_menu;
pub mod create_menu;
pub mod dimension_edit;
pub mod display_quality;
pub(crate) mod display_states;
pub mod document;
#[cfg(target_arch = "wasm32")]
pub mod documents_browser;
pub mod drag;
pub mod drawing;
pub mod feature_forms;
pub mod feature_panel;
pub mod file;
pub mod fillet_collect;
pub mod form_bar;
pub mod formula_help;
pub mod geometry_message;
pub mod graphics;
mod grid;
pub mod history_panel;
pub mod hole_wizard_panel;
pub mod icons;
pub mod import;
pub mod in_context;
pub mod interference;
pub mod journal;
#[cfg(target_arch = "wasm32")]
pub mod journal_browser;
pub mod kernel_worker;
pub mod left_bar;
pub mod locations;
mod loft_panel;
pub(crate) mod material_dialog;
pub mod mates;
pub mod measure;
mod multibody;
pub mod navigation;
pub mod options_bar;
pub mod palette;
pub mod panels;
pub mod part_check;
pub mod platform;
pub mod pmi;
pub mod preferences;
pub mod print_check;
pub mod properties;
pub mod recent;
pub mod recovery;
pub mod review;
mod ring_menu;
pub mod scan;
pub mod scene_import;
pub mod section;
pub mod settings;
mod shortcut_menu;
pub mod sketch_diagnostics;
pub mod sketch_tools;
pub mod sketch_values;
pub mod sketcher;
pub mod start_page;
pub mod startup;
pub mod state;
pub mod subassembly;
pub mod sweep_panel;
pub mod theme;
pub mod tool_palette;
pub mod top_bar;
pub mod tree;
pub mod tree_visibility;
pub mod values_tab;
pub mod viewport;
#[cfg(target_arch = "wasm32")]
pub mod web;
mod zip;

/// The sketch model, solver and profiles: the plumbing crate the sketcher draws on.
pub use soliddraft_sketch as sketch;
