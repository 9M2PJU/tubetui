//! tubetui — YouTube in your terminal.
//!
//! The core is pure and synchronous: [`app::State::update`] turns an
//! [`app::Action`] into state changes plus [`app::Effect`]s, and the binary
//! performs the effects (spawning mpv, running searches). Everything that
//! touches processes or the terminal stays at the edges.

pub mod app;
pub mod config;
pub mod domain;
pub mod keymap;
pub mod media;
pub mod player;
pub mod session;
pub mod store;
pub mod ui;
pub mod youtube;
