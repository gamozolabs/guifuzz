pub mod winbindings;
pub mod rng;

use std::error::Error;
use std::collections::{HashSet, HashMap};
use std::sync::{Mutex, Arc};
pub use rng::Rng;
pub use winbindings::Window;

/// Sharable fuzz input
pub type FuzzInput = Arc<Vec<FuzzerAction>>;

/// Fuzz case statistics
#[derive(Default)]
pub struct Statistics {
    /// Number of fuzz cases
    pub fuzz_cases: u64,

    /// Coverage database. Maps (module, offset) to `FuzzInput`s
    pub coverage_db: HashMap<(Arc<String>, usize), FuzzInput>,

    /// Set of all unique inputs
    pub input_db: HashSet<FuzzInput>,

    /// List of all unique inputs
    pub input_list: Vec<FuzzInput>,

    /// Unique set of fuzzer actions
    pub unique_action_set: HashSet<FuzzerAction>,

    /// List of all unique fuzzer actions
    pub unique_actions: Vec<FuzzerAction>,

    /// Number of crashes
    pub crashes: u64,

    /// Database of crash file names to `FuzzInput`s
    pub crash_db: HashMap<String, FuzzInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FuzzerAction {
    LeftClick { idx: usize },
    Close,
    MenuAction { menu_id: u32 },
    KeyPress { key: usize },
}

pub fn perform_actions(pid: u32,
        actions: &[FuzzerAction]) -> Result<(), Box<dyn Error>>{
    // Attach to the Calculator window
    let primary_window = Window::attach_pid(pid, "Calculator")?;

    for &action in actions {
        match action {
            FuzzerAction::LeftClick { idx } => {
                // Click on the GUI element
                let sub_windows = primary_window.enumerate_subwindows();
                if sub_windows.is_err() {
                    return Ok(());
                }
                let sub_windows = sub_windows.unwrap();

                if let Some(window) = sub_windows.get(idx) {
                    let _ = window.left_click(None);
                }
            }
            FuzzerAction::Close => {
                let _ = primary_window.close();
            }
            FuzzerAction::MenuAction { menu_id } => {
                // Select a random menu item and click it
                let _ = primary_window.use_menu_id(menu_id);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            FuzzerAction::KeyPress { key } => {
                // Press a key on the keyboard
                let _ = primary_window.press_key(key);
            }
        }
    }

    Ok(())
}

pub fn mutate(stats: Arc<Mutex<Statistics>>)
        -> Result<Vec<FuzzerAction>, Box<dyn Error>> {
    // Create a new RNG
    let rng = Rng::new();

    // Get access to the global database
    let stats = stats.lock().unwrap();

    // Pick an input to use as the basis of this fuzz case
    let input_sel = rng.rand() % stats.input_list.len();
    let mut input: Vec<FuzzerAction> = (*stats.input_list[input_sel]).clone();

    // Make up to n modifications, minimum of one
    for _ in 0..((rng.rand() & 0x1f) + 1) {
        let sel = rng.rand() % 5;

        match sel {
            0 => {
                // Splice in a random portion from an existing input

                // Select a random slice from our current input
                if input.len() == 0 { continue; }
                let inp_start  = rng.rand() % input.len();
                let inp_length = rng.rand() % (rng.rand() % 64 + 1);
                let inp_end    = std::cmp::min(inp_start + inp_length,
                    input.len());

                // Select a random slice from a random input
                let donor_idx    = rng.rand() % stats.input_list.len();
                let donor_input  = &stats.input_list[donor_idx];
                if donor_input.len() == 0 { continue; }

                let donor_start  = rng.rand() % donor_input.len();
                let donor_length = rng.rand() % (rng.rand() % 64 + 1);
                let donor_end = std::cmp::min(donor_start + donor_length,
                                                 donor_input.len());

                // Spice in the donor input contents into the input
                input.splice(inp_start..inp_end, 
                    donor_input[donor_start..donor_end]
                    .iter().cloned());
            }
            1 => {
                // Delete a random portion from the input

                // Select a random slice from our current input
                if input.len() == 0 { continue; }
                let inp_start  = rng.rand() % input.len();
                let inp_length = rng.rand() % (rng.rand() % 64 + 1);
                let inp_end    = std::cmp::min(inp_start + inp_length,
                    input.len());

                // Delete this slice from the input
                input.splice(inp_start..inp_end, [].iter().cloned());
            }
            2 => {
                // Repeat a certain part of the slice many times
                if input.len() == 0 { continue; }
                let sel = rng.rand() % input.len();
                for _ in 0..rng.rand() % (rng.rand() % 64 + 1) {
                    input.insert(sel, input[sel]);
                }
            }
            3 => {
                // Insert a random slice into the vector
                
                // Select a random index from our current input
                if input.len() == 0 { continue; }
                let inp_index = rng.rand() % input.len();

                // Select a random slice from a random input
                let donor_idx    = rng.rand() % stats.input_list.len();
                let donor_input  = &stats.input_list[donor_idx];
                if donor_input.len() == 0 { continue; }
                let donor_start  = rng.rand() % donor_input.len();
                let donor_length = rng.rand() % (rng.rand() % 64 + 1);
                let donor_end = std::cmp::min(donor_start + donor_length,
                                              donor_input.len());

                // Splice in donor slice into `inp_index` in the input
                let new_inp: Vec<FuzzerAction> = input[0..inp_index].iter()
                    .chain(donor_input[donor_start..donor_end].iter())
                    .chain(input[inp_index..].iter()).cloned().collect();

                // Replace the input with this newly created input
                input = new_inp;
            }
            4 => {
                if stats.unique_actions.len() == 0 ||
                    input.len() == 0 { continue; }

                // Get a random action
                let rand_action = stats.unique_actions[
                    rng.rand() % stats.unique_actions.len()];

                // Add the action to the input
                input.insert(rng.rand() % input.len(), rand_action);
            }
            _ => panic!("Unreachable"),
        }
    }

    Ok(input)
}

pub fn generator(pid: u32) -> Result<Vec<FuzzerAction>, Box<dyn Error>> {
    // Log of all actions performed
    let mut actions = Vec::new();

    // Create an RNG
    let rng = Rng::new();

    // Attach to the Calculator window
    let primary_window = Window::attach_pid(pid, "Calculator")?;

    loop {
        {
            // Pick a random GUI element to click on
            let sub_windows = primary_window.enumerate_subwindows();
            if sub_windows.is_err() {
                return Ok(actions);
            }
            let sub_windows = sub_windows.unwrap();

            let sel = rng.rand() % sub_windows.len();
            let window = sub_windows[sel];

            // Click on the GUI element
            actions.push(FuzzerAction::LeftClick { idx: sel });
            let _ = window.left_click(None);
        }

        {
            // Press a random key on the keyboard
            let key = ((rng.rand() % 10) as u8 + b'0') as usize;
            actions.push(FuzzerAction::KeyPress { key });
            let _ = primary_window.press_key(key);
        }

        if rng.rand() & 0x1f == 0 {
            // Press a random key on the keyboard
            let key = rng.rand() as u8 as usize;
            actions.push(FuzzerAction::KeyPress { key });
            let _ = primary_window.press_key(key);
        }

        // Chance of randomly closing the application
        if (rng.rand() & 0xff) == 0 {
            actions.push(FuzzerAction::Close);
            let _ = primary_window.close();
        }

        // Chance of randomly clicking a menu item
        if (rng.rand() & 0x1f) == 0 {
            if let Ok(menus) = primary_window.enum_menus() {
                // Get a list of all of the menu items in calc
                let menus: Vec<u32> = menus.iter().cloned().collect();

                // Select a random menu item and click it
                let sel = menus[rng.rand() % menus.len()];
                actions.push(FuzzerAction::MenuAction { menu_id: sel });
                let _ = primary_window.use_menu_id(sel);

                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}

