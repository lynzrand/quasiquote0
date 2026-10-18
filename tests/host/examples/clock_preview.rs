use quasiquote0_host_tests::clock_face::{self, ClockFace};

fn main() {
    let frame = clock_face::render(ClockFace::Minute {
        hour: 20,
        minute: 18,
    });
    let path = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/clock-preview.bin")
        });
    std::fs::write(path, frame.as_bytes()).unwrap();
}
