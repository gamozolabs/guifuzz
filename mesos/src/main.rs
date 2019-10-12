extern crate debugger;
extern crate guifuzz;

pub mod mesofile;

use std::path::Path;
use std::process::Command;
use std::collections::{HashMap};
use std::sync::{Arc, Mutex};
use std::fs::File;
use std::io::Write;
use std::time::{Instant, Duration};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use debugger::{ExitType, Debugger};
use guifuzz::*;

fn record_input(fuzz_input: FuzzInput) {
    let mut hasher = DefaultHasher::new();
    fuzz_input.hash(&mut hasher);

    let _ = std::fs::create_dir("inputs");
    std::fs::write(format!("inputs/{:016x}.input", hasher.finish()),
        format!("{:#?}", fuzz_input)).expect("Failed to save input to disk");
}

fn worker(stats: Arc<Mutex<Statistics>>) {
    // Local stats database
    let mut local_stats = Statistics::default();

    // Create an RNG for this thread
    let rng = Rng::new();

    loop {
        // Delete all state invoked with the calc.exe process
        Command::new("reg.exe").args(&[
            "delete",
            r"HKEY_CURRENT_USER\Software\Microsoft\Calc",
            "/f",
        ]).output().unwrap();

        std::thread::sleep(Duration::from_millis(rng.rand() as u64 % 500));

        // Create a new calc instance
        let mut dbg = Debugger::spawn_proc(&["calc.exe".into()], false);

        // Load the meso
        mesofile::load_meso(&mut dbg, Path::new("calc.exe.meso"));

        // Spin up the fuzzer thread
        let pid = dbg.pid;
        let thr = {
            let generate = (rng.rand() & 0x7) == 0;
            let stats = stats.clone();

            std::thread::spawn(move || {
                while Window::attach_pid(pid, "Calculator").is_err() {
                    std::thread::sleep(Duration::from_millis(200));
                }

                if generate || stats.lock().unwrap().input_db.len() == 0 {
                    generator(pid).unwrap_or(Vec::new())
                } else {
                    let mutated = mutate(stats).unwrap_or(Vec::new());
                    let _ = perform_actions(pid, &mutated);
                    mutated
                }
            })
        };

        // Debug forever
        let exit_state = dbg.run();

        // Extra-kill the debuggee
        let _ = dbg.kill();

        // Swap coverage with the debugger and drop it so that the debugger
        // disconnects its resources from the debuggee so it can exit
        let mut coverage = HashMap::new();
        std::mem::swap(&mut dbg.coverage, &mut coverage);
        std::mem::drop(dbg);

        // Connect to the fuzzer thread and get the result
        let genres = thr.join();
        if genres.is_err() {
            continue;
        }
        let genres = genres.unwrap();

        // Wrap up the fuzz input in an `Arc`
        let fuzz_input = Arc::new(genres);

        // Go through all coverage entries in the coverage database
        for (_, (module, offset, _, _)) in coverage.iter() {
            let key = (module.clone(), *offset);

            // Check if this coverage entry is something we've never seen
            // before
            if !local_stats.coverage_db.contains_key(&key) {
                // Coverage entry is new, save the fuzz input in the input
                // database
                local_stats.input_db.insert(fuzz_input.clone());

                // Update the module+offset in the coverage database to
                // reflect that this input caused this coverage to occur
                local_stats.coverage_db.insert(key.clone(),
                    fuzz_input.clone());

                // Get access to global stats
                let mut stats = stats.lock().unwrap();
                if !stats.coverage_db.contains_key(&key) {
                    // Save input to global input database
                    if stats.input_db.insert(fuzz_input.clone()) {
                        stats.input_list.push(fuzz_input.clone());
                
                        record_input(fuzz_input.clone());

                        // Update the action database with known-feasible
                        // actions
                        for &action in fuzz_input.iter() {
                            if stats.unique_action_set.insert(action) {
                                stats.unique_actions.push(action);
                            }
                        }
                    }
                    
                    // Save coverage to global coverage database
                    stats.coverage_db.insert(key.clone(), fuzz_input.clone());
                }
            }
        }

        // Get access to global stats
        let mut stats = stats.lock().unwrap();

        // Update fuzz case count
        local_stats.fuzz_cases += 1;
        stats.fuzz_cases += 1;

        // Check if this case ended due to a crash
        if let ExitType::Crash(crashname) = exit_state {
            // Update crash information
            local_stats.crashes += 1;
            stats.crashes       += 1;

            // Add the crashing input to the input databases
            local_stats.input_db.insert(fuzz_input.clone());
            if stats.input_db.insert(fuzz_input.clone()) {
                stats.input_list.push(fuzz_input.clone());

                record_input(fuzz_input.clone());

                // Update the action database with known-feasible
                // actions
                for &action in fuzz_input.iter() {
                    if stats.unique_action_set.insert(action) {
                        stats.unique_actions.push(action);
                    }
                }
            }

            // Add the crash name and corresponding fuzz input to the crash
            // database
            local_stats.crash_db.insert(crashname.clone(), fuzz_input.clone());
            stats.crash_db.insert(crashname, fuzz_input.clone());
        }
    }
}

fn main() {
    // Global statistics
    let stats = Arc::new(Mutex::new(Statistics::default()));

    // Open a log file
    let mut log = File::create("fuzz_stats.txt").unwrap();

    // Save the current time
    let start_time = Instant::now();

    for _ in 0..10 {
        // Spawn threads
        let stats = stats.clone();
        let _ = std::thread::spawn(move || {
            worker(stats);
        });
    }

    loop {
        std::thread::sleep(Duration::from_millis(1000));

        // Get access to the global stats
        let stats = stats.lock().unwrap();

        let uptime = (Instant::now() - start_time).as_secs_f64();
        let fuzz_case = stats.fuzz_cases;
        print!("{:12.2} uptime | {:7} fuzz cases | {:5} uniq actions | \
                {:8} coverage | {:5} inputs | {:6} crashes [{:6} unique]\n",
            uptime, fuzz_case,
            stats.unique_actions.len(),
            stats.coverage_db.len(), stats.input_db.len(),
            stats.crashes, stats.crash_db.len());

        write!(log, "{:12.0} {:7} {:8} {:5} {:6} {:6}\n",
            uptime, fuzz_case, stats.coverage_db.len(), stats.input_db.len(),
            stats.crashes, stats.crash_db.len()).unwrap();
        log.flush().unwrap();
    }
}

