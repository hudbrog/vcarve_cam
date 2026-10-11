fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "geometry") {
        println!("{}", sketcher_feasibility::geometry::report());
    } else {
        let input = std::fs::read_to_string(&args[1]).expect("fixture file");
        let output = sketcher_feasibility::run_json(&input).expect("qualification run");
        if let Some(path) = args.get(2) {
            std::fs::write(path, output).unwrap();
        } else {
            println!("{output}");
        }
    }
}
