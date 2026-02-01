//! System tray icon management

use tray_icon::{
    menu::{Menu, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

/// Tray icon manager
pub struct TrayManager {
    _tray: TrayIcon,
    menu_show: MenuItem,
    menu_quit: MenuItem,
}

impl TrayManager {
    /// Create new tray manager
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Create menu items
        let menu_show = MenuItem::new("Show Window", true, None);
        let menu_quit = MenuItem::new("Quit", true, None);

        // Create menu
        let menu = Menu::new();
        menu.append(&menu_show)?;
        menu.append(&menu_quit)?;

        // Load icon from embedded bytes
        let icon_bytes = include_bytes!("../../../icons/icon.png");
        let icon_image = image::load_from_memory(icon_bytes)?;
        let icon_rgba = icon_image.to_rgba8();
        let (width, height) = icon_rgba.dimensions();

        let icon = Icon::from_rgba(
            icon_rgba.into_raw(),
            width,
            height,
        )?;

        // Build tray icon
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Convocations")
            .with_icon(icon)
            .build()?;

        Ok(Self {
            _tray: tray,
            menu_show,
            menu_quit,
        })
    }

    /// Handle menu events, returns true if should quit
    pub fn handle_events(&self, window_visible: &mut bool) -> bool {
        use tray_icon::menu::MenuEvent;

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.menu_show.id() {
                *window_visible = true;
                return false;
            } else if event.id == self.menu_quit.id() {
                return true;
            }
        }

        false
    }
}
