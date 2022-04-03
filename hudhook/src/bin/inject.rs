use console::Term;
use dialoguer::Select;
use hudhook::inject;

fn main() {
    let term = Term::stdout();

    let mut exe_path = std::env::current_exe().unwrap();
    exe_path.pop();

    let dll = {
        let mut dlls = std::fs::read_dir(exe_path)
            .unwrap()
            .filter_map(|f| f.ok())
            .filter_map(|f| {
                let path = f.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if Some("dll") == ext {
                    Some(path)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let dlls_print = dlls.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>();

        let dll = Select::new()
            .items(&dlls_print)
            .default(0)
            .with_prompt("DLL to inject:")
            .interact_on(&term)
            .map_err(|e| format!("{}", e))
            .unwrap();

        dlls.remove(dll)
    };

    inject::inject("DARK SOULS III", dll);
}
