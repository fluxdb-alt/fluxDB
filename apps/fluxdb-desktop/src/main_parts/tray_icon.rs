use std::cell::RefCell;

use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu as TrayMenu, MenuEvent, MenuItem as TrayMenuItem, PredefinedMenuItem},
};

const TRAY_OPEN_ID: &str = "gdb.tray.open";
const TRAY_QUIT_ID: &str = "gdb.tray.quit";

thread_local! {
    static TRAY_ICON: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    static TRAY_EVENT_TASK: RefCell<Option<Task<()>>> = const { RefCell::new(None) };
}

fn install_tray_icon(cx: &mut App) {
    TRAY_ICON.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }

        match create_tray_icon() {
            Ok(icon) => *slot.borrow_mut() = Some(icon),
            Err(err) => tracing::warn!(target: "fluxdb_desktop", error = %err, "初始化系统托盘图标失败"),
        }
    });
    install_tray_menu_event_task(cx);
}

fn create_tray_icon() -> anyhow::Result<TrayIcon> {
    let icon = load_tray_icon()?;
    let menu = create_tray_menu()?;
    let mut builder = TrayIconBuilder::new()
        .with_tooltip("FluxDB")
        .with_menu(Box::new(menu))
        .with_icon(icon);

    #[cfg(target_os = "macos")]
    {
        builder = builder.with_icon_as_template(true);
    }

    Ok(builder.build()?)
}

fn create_tray_menu() -> anyhow::Result<TrayMenu> {
    let menu = TrayMenu::new();
    let open = TrayMenuItem::with_id(TRAY_OPEN_ID, "打开", true, None);
    let quit = TrayMenuItem::with_id(TRAY_QUIT_ID, "退出", true, None);
    let separator = PredefinedMenuItem::separator();
    menu.append_items(&[&open, &separator, &quit])?;
    Ok(menu)
}

fn install_tray_menu_event_task(cx: &mut App) {
    TRAY_EVENT_TASK.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }

        let task = cx.spawn(async move |cx| {
            loop {
                smol::Timer::after(Duration::from_millis(150)).await;
                cx.update(handle_tray_menu_events);
            }
        });
        *slot.borrow_mut() = Some(task);
    });
}

fn handle_tray_menu_events(cx: &mut App) {
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        match event.id().as_ref() {
            TRAY_OPEN_ID => activate_main_window(cx),
            TRAY_QUIT_ID => cx.quit(),
            _ => {}
        }
    }
}

fn activate_main_window(cx: &mut App) {
    cx.activate(true);
    if let Some(window) = cx
        .window_stack()
        .and_then(|windows| windows.into_iter().next())
        .or_else(|| cx.windows().into_iter().next())
    {
        let _ = window.update(cx, |_, window, _| window.activate_window());
    }
}

fn install_close_to_tray(window: &Window, cx: &mut App) {
    window.on_window_should_close(cx, |_, cx| {
        #[cfg(target_os = "macos")]
        cx.hide();

        false
    });
}

fn load_tray_icon() -> anyhow::Result<Icon> {
    let image = image::load_from_memory(tray_icon_png())?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

#[cfg(target_os = "macos")]
fn tray_icon_png() -> &'static [u8] {
    include_bytes!("../../assets/status-icon-gdb-v3.png")
}

#[cfg(not(target_os = "macos"))]
fn tray_icon_png() -> &'static [u8] {
    include_bytes!("../../assets/app-icon-tray.png")
}
