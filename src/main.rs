mod app;
mod config;
mod spice;
mod ui;
mod vm;

fn main() {
    env_logger::init();
    app::run();
}
