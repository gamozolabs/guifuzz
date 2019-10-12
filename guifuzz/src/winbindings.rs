use std::io;
use std::fmt;
use std::error::Error;
use std::convert::TryInto;
use std::ops::Deref;
use std::collections::BTreeSet;

/// Callback function for `EnumChildWindows()`
type EnumChildProc = extern "C" fn(hwnd: usize, lparam: usize) -> bool;

/// Callback function for `EnumWindows()`
type EnumWindowsProc = extern "C" fn (hwnd: usize, lparam: usize) -> bool;

#[link(name="User32")]
extern "system" {
    fn FindWindowW(lpClassName: *mut u16, lpWindowName: *mut u16) -> usize;
    fn EnumChildWindows(hwnd: usize, func: EnumChildProc,
        lparam: usize) -> bool;
    fn GetWindowTextW(hwnd: usize, string: *mut u16, chars: i32) -> i32;
    fn GetWindowTextLengthW(hwnd: usize) -> i32;
    fn PostMessageW(hwnd: usize, msg: u32, wparam: usize, lparam: usize)
        -> bool;
    fn GetMenu(hwnd: usize) -> usize;
    fn GetSubMenu(hwnd: usize, pos: i32) -> usize;
    fn GetMenuItemID(menu: usize, pos: i32) -> u32;
    fn GetMenuItemCount(menu: usize) -> i32;
    fn EnumWindows(func: EnumWindowsProc, lparam: usize) -> bool;
    fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Rect {
    left:   i32,
    top:    i32,
    right:  i32,
    bottom: i32,
}

/// Convert a Rust UTF-8 `string` into a NUL-terminated UTF-16 vector
fn str_to_utf16(string: &str) -> Vec<u16> {
    let mut ret: Vec<u16> = string.encode_utf16().collect();
    ret.push(0);
    ret
}

/// An active handle to a window
#[derive(Clone, Copy)]
pub struct Window {
    /// Handle to the window which we have opened
    hwnd: usize,
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Window {{ hwnd: {:#x}, title: \"{}\" }}",
            self.hwnd, self.window_text().unwrap())
    }
}

/// Structure which contains a listing of all child windows
#[derive(Default, Debug)]
pub struct WindowListing {
    /// List of all window HWNDs
    windows: Vec<Window>,
}

impl Deref for WindowListing {
    type Target = [Window];

    fn deref(&self) -> &Self::Target {
        &self.windows
    }
}

/// Different message types to be sent to `PostMessage()` and `SendMessage()`
#[repr(u32)]
enum MessageType {
    /// Left mouse button down event
    LButtonDown = 0x0201,

    /// Left mouse button up event
    LButtonUp = 0x0202,

    /// Sends a key down event to the window
    KeyDown = 0x0100,

    /// Sends a key up event to the window
    KeyUp = 0x0101,

    /// Sends a command to a window, typically sent when a button is pressed
    /// or a menu item is used
    Command = 0x0111,

    /// Sends a graceful exit to the window
    Close = 0x0010,
}

/// Different types of virtual key codes
#[repr(usize)]
pub enum VirtualKeyCode {
    Left  = 0x25,
    Up    = 0x26,
    Right = 0x27,
    Down  = 0x28,
    F10   = 0x79,
}

/// Rust implementation of `MENUITEMINFOW`
#[repr(C)]
#[derive(Debug, Default)]
struct MenuItemInfo {
    size:          u32,
    mask:          u32,
    typ:           u32,
    state:         u32,
    id:            u32,
    sub_menu:      usize,
    bmp_checked:   usize,
    bmp_unchecked: usize,
    item_data:     usize,
    type_data:     usize,
    cch:           u32,
    bmp_item:      usize,
}

impl Window {
    /// Find a window with `title`, and return a new `Window` object
    pub fn attach(title: &str) -> io::Result<Self> {
        // Convert the title to UTF-16
        let mut title = str_to_utf16(title); 

        // Finds the window with `title`
        let ret = unsafe {
            FindWindowW(std::ptr::null_mut(), title.as_mut_ptr())
        };

        if ret != 0 {
            // Successfully got a handle to the window
            return Ok(Window {
                hwnd: ret,
            });
        } else {
            // FindWindow() failed, return out the corresponding error
            Err(io::Error::last_os_error())
        }
    }

    extern "C" fn enum_windows_handler(hwnd: usize, lparam: usize) -> bool {
        let param = unsafe {
            &mut *(lparam as *mut (u32, Option<usize>, String))
        };

        let mut pid = 0;
        let tid = unsafe{
            GetWindowThreadProcessId(hwnd, &mut pid)
        };
        if pid == 0 || tid == 0 {
            return true;
        }

        if param.0 == pid {
            // Create a window for this window we are enumerating
            let tmpwin = Window { hwnd };
            
            // Get the title for the window
            if let Ok(title) = tmpwin.window_text() {
                // Check if the title matches what we are searching for
                if &title == &param.2 {
                    // Match!
                    param.1 = Some(hwnd);
                }
            }

            // Keep enumerating
            true
        } else {
            // Keep enumerating
            true
        }
    }

    /// Return a `Window` object for the `pid`s main window
    pub fn attach_pid(pid: u32, window_title: &str) -> io::Result<Self> {
        let mut context: (u32, Option<usize>, String) =
            (pid, None, window_title.into());

        unsafe {
            if !EnumWindows(Self::enum_windows_handler,
                    &mut context as *mut _ as usize) {
                // EnumWindows() failed, return out the corresponding error
                return Err(io::Error::last_os_error());
            }
        }

        if let Some(hwnd) = context.1 {
            // Create the window object
            Ok(Window { hwnd })
        } else {
            // Could not find a HWND
            Err(io::Error::new(io::ErrorKind::Other,
                "Could not find HWND for pid"))
        }
    }

    /// Internal callback for `EnumChildWindows()` used from the
    /// `enumerate_subwindows()` member function
    extern "C" fn enum_child_window_callback(hwnd: usize, lparam: usize)
            -> bool {
        // Get the parameter we passed in
        let listing: &mut WindowListing = unsafe {
            &mut *(lparam as *mut WindowListing)
        };

        // Add this window handle to the listing
        listing.windows.push(Window { hwnd });

        // Continue the search
        true
    }

    /// Enumerate all of the sub-windows belonging to `Self` recursively
    pub fn enumerate_subwindows(&self) -> io::Result<WindowListing> {
        // Create a new, empty window listing
        let mut listing = WindowListing::default();

        unsafe {
            // Enumerate all the child windows
            if EnumChildWindows(self.hwnd, 
                    Self::enum_child_window_callback,
                    &mut listing as *mut WindowListing as usize) {
                // Child windows successfully enumerated
                Ok(listing)
            } else {
                // Failure during call to `EnumChildWindows()`
                Err(io::Error::last_os_error())
            }
        }
    }

    /// Gets the title for the window, or in the case of a control field, gets
    /// the text on the object
    pub fn window_text(&self) -> Result<String, Box<dyn Error>> {
        let text_len = unsafe { GetWindowTextLengthW(self.hwnd) };

        // Return an empty string if the window text length was reported as
        // zero
        if text_len == 0 {
            return Ok(String::new());
        }

        // Allocate a buffer to hold `text_len` wide characters
        let text_len: usize = text_len.try_into().unwrap();
        let alc_len = text_len.checked_add(1).unwrap();
        let mut wchar_buffer: Vec<u16> = Vec::with_capacity(alc_len);

        unsafe {
            // Get the window text
            let ret = GetWindowTextW(self.hwnd, wchar_buffer.as_mut_ptr(),
                alc_len.try_into().unwrap());

            // Set the length of the vector
            wchar_buffer.set_len(ret.try_into().unwrap());
        }

        // Convert the UTF-16 string into a Rust UTF-8 `String`
        String::from_utf16(wchar_buffer.as_slice()).map_err(|x| {
            x.into()
        })
    }

    /// Does a left click of the current window
    pub fn left_click(&self, state: Option<KeyMouseState>) -> io::Result<()> {
        // Get the state, or create a new, empty state
        let mut state = state.unwrap_or_default();

        unsafe {
            state.left_mouse = true;
            if !PostMessageW(self.hwnd, MessageType::LButtonDown as u32,
                    state.into(), 0) {
                // PostMessageW() failed
                return Err(io::Error::last_os_error());
            }

            state.left_mouse = false;
            if !PostMessageW(self.hwnd, MessageType::LButtonUp as u32,
                    state.into(), 0) {
                // PostMessageW() failed
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    /// Presses a key down and releases it
    pub fn press_key(&self, key: usize) -> io::Result<()> {
        unsafe {
            if !PostMessageW(self.hwnd, MessageType::KeyDown as u32, key, 0) {
                // PostMessageW() failed
                return Err(io::Error::last_os_error());
            }

            if !PostMessageW(self.hwnd, MessageType::KeyUp as u32, key,
                    3 << 30) {
                // PostMessageW() failed
                return Err(io::Error::last_os_error());
            }
        }
        
        Ok(())
    }

    /// Recurse into a menu listing, looking for sub menus
    fn recurse_menu(&self, menu_ids: &mut BTreeSet<u32>, menu_handle: usize)
            -> io::Result<()> {
        unsafe {
            // Get the number of menu items
            let menu_count = GetMenuItemCount(menu_handle);
            if menu_count == -1 {
                // GetMenuItemCount() failed
                return Err(io::Error::last_os_error());
            }

            // Go through each item in the menu
            for menu_index in 0..menu_count {
                // Get the menu ID
                let menu_id = GetMenuItemID(menu_handle, menu_index);

                if menu_id == !0 {
                    // Menu is a sub menu, get the sub menu handle
                    let sub_menu = GetSubMenu(menu_handle, menu_index);
                    if sub_menu == 0 {
                        // GetSubMenu() failed
                        return Err(io::Error::last_os_error());
                    }

                    // Recurse into the sub-menu
                    self.recurse_menu(menu_ids, sub_menu)?;
                } else {
                    // This is a menu identifier, add it to the set
                    menu_ids.insert(menu_id);
                }
            }

            Ok(())
        }
    }

    /// Enumerate all window menus, return a set of the menu IDs which can
    /// be used with a `WM_COMMAND` message
    pub fn enum_menus(&self) -> io::Result<BTreeSet<u32>> {
        // Get the window's main menu
        let menu = unsafe { GetMenu(self.hwnd) };
        if menu == 0 {
            // GetMenu() error
            return Err(io::Error::last_os_error());
        }

        // Create the empty hash set
        let mut menu_ids = BTreeSet::new();

        // Recursively search through the menu
        self.recurse_menu(&mut menu_ids, menu)?;

        Ok(menu_ids)
    }

    /// Send a message to the window, indicating that `menu_id` was clicked.
    /// To get a valid `menu_id`, use the `enum_menus` member function.
    pub fn use_menu_id(&self, menu_id: u32) -> io::Result<()> {
        unsafe {
            if PostMessageW(self.hwnd, MessageType::Command as u32,
                    menu_id.try_into().unwrap(), 0) {
                // Success!
                Ok(())
            } else {
                // PostMessageW() error
                Err(io::Error::last_os_error())
            }
        }
    }

    /// Attempts to gracefully close the applications
    pub fn close(&self) -> io::Result<()> {
        unsafe {
            if PostMessageW(self.hwnd, MessageType::Close as u32, 0, 0) {
                // Success!
                Ok(())
            } else {
                // PostMessageW() error
                Err(io::Error::last_os_error())
            }
        }
    }
}

/// Holds the state of some of the special keyboard and mouse buttons during
/// certain mouse events
#[derive(Default, Debug, Clone, Copy)]
pub struct KeyMouseState {
    /// Left mouse button is down
    pub left_mouse: bool,

    /// Middle mouse button is down
    pub middle_mouse: bool,

    /// Right mouse button is down
    pub right_mouse: bool,

    /// Shift key is down
    pub shift: bool,

    /// First x button is down
    pub xbutton1: bool,

    /// Second x button is down
    pub xbutton2: bool,

    /// Control key is down
    pub control: bool,
}


impl Into<usize> for KeyMouseState {
    fn into(self) -> usize {
        (if self.left_mouse   { 0x0001 } else { 0 }) |
        (if self.middle_mouse { 0x0010 } else { 0 }) |
        (if self.right_mouse  { 0x0002 } else { 0 }) |
        (if self.shift        { 0x0004 } else { 0 }) |
        (if self.xbutton1     { 0x0020 } else { 0 }) |
        (if self.xbutton2     { 0x0040 } else { 0 }) |
        (if self.control      { 0x0008 } else { 0 })
    }
}

