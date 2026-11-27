//! 桌面应用宿主层：共享状态、全局事件、通知、窗口、灵动岛、托盘与后台线程。

pub(crate) mod background;
pub(crate) mod events;
pub(crate) mod island;
pub(crate) mod main_window;
pub(crate) mod notify;
pub(crate) mod startup;
pub mod state;
pub(crate) mod toast_link;
pub(crate) mod tray;
pub(crate) mod watcher;
pub(crate) mod windows;
