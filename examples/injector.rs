use std::path::PathBuf;

use hudhook::inject::Process;

fn main() {
    let mut args = std::env::args();
    args.next().unwrap();
    let name = args.next().unwrap();
    let dll: PathBuf = args.next().unwrap().into();
    let process = Process::by_name(&name).expect("Process by name");
    process.inject(dll).expect("Inject");
}
