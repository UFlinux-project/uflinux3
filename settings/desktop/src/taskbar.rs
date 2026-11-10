//! Window grouping and taskbar activation without GUI or compositor ownership.
use std::collections::HashMap;

pub type WindowId = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Desktop file ID, including `.desktop`.
    pub desktop_id: String,
    pub name: String,
    pub icon: String,
    pub startup_wm_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub app_id: String,
    pub title: String,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Canonical desktop ID when known, otherwise the compositor's app ID.
    pub id: String,
    pub name: String,
    pub icon: String,
    pub pinned: bool,
    pub windows: Vec<Window>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    Launch(String),
    Activate(WindowId),
    Minimize(WindowId),
}

/// An ordered list of favorites and first-seen running applications.
#[derive(Default)]
pub struct Taskbar {
    apps: Vec<Application>,
    pinned: Vec<String>,
    windows: HashMap<WindowId, Window>,
    order: Vec<WindowId>,
}

fn key(value: &str) -> String {
    value.trim().trim_end_matches(".desktop").to_lowercase()
}

impl Taskbar {
    pub fn new(pinned: Vec<String>) -> Self {
        let mut result = Self::default();
        for id in pinned {
            if !result.pinned.iter().any(|item| key(item) == key(&id)) {
                result.pinned.push(id);
            }
        }
        result
    }

    /// Replace the catalog after a desktop-file change, regrouping existing windows.
    pub fn set_applications(&mut self, apps: Vec<Application>) {
        self.apps = apps;
    }

    /// Insert or update one compositor window; its original ordering is retained.
    pub fn upsert_window(&mut self, window: Window) {
        if !self.windows.contains_key(&window.id) {
            self.order.push(window.id);
        }
        self.windows.insert(window.id, window);
    }

    pub fn remove_window(&mut self, id: WindowId) {
        if self.windows.remove(&id).is_some() {
            self.order.retain(|window| *window != id);
        }
    }

    fn app_for(&self, app_id: &str) -> Option<&Application> {
        let wanted = key(app_id);
        if wanted.is_empty() {
            return None;
        }
        // Exact desktop IDs take precedence over StartupWMClass aliases.
        self.apps
            .iter()
            .find(|app| key(&app.desktop_id) == wanted)
            .or_else(|| {
                self.apps.iter().find(|app| {
                    app.startup_wm_class
                        .as_deref()
                        .is_some_and(|class| key(class) == wanted)
                })
            })
    }

    fn group(&self, app_id: &str, pinned: bool) -> Group {
        let app = self.app_for(app_id);
        let id = app
            .map(|item| item.desktop_id.clone())
            .unwrap_or_else(|| app_id.to_owned());
        Group {
            name: app
                .map(|item| item.name.clone())
                .unwrap_or_else(|| app_id.to_owned()),
            icon: app
                .map(|item| item.icon.clone())
                .filter(|icon| !icon.is_empty())
                .unwrap_or_else(|| "application-x-executable".into()),
            id,
            pinned,
            windows: Vec::new(),
        }
    }

    pub fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = self.pinned.iter().map(|id| self.group(id, true)).collect();
        for id in &self.order {
            let Some(window) = self.windows.get(id) else {
                continue;
            };
            // An empty app ID is not a shared application: keep these windows separate.
            if window.app_id.trim().is_empty() {
                let mut group = self.group(&format!("window-{}", window.id), false);
                group.name = if window.title.is_empty() {
                    "Application".into()
                } else {
                    window.title.clone()
                };
                group.windows.push(window.clone());
                groups.push(group);
                continue;
            }
            let candidate = self.group(&window.app_id, false);
            if let Some(group) = groups
                .iter_mut()
                .find(|group| key(&group.id) == key(&candidate.id))
            {
                group.windows.push(window.clone());
            } else {
                let mut group = candidate;
                group.windows.push(window.clone());
                groups.push(group);
            }
        }
        groups
    }
}

impl Group {
    /// Match Ironbar's default: focus a non-focused window first, otherwise
    /// minimize the focused window; a closed favorite launches its desktop file.
    pub fn activate(&self) -> Option<Activation> {
        if let Some(window) = self.windows.iter().find(|window| !window.focused) {
            Some(Activation::Activate(window.id))
        } else if let Some(window) = self.windows.first() {
            Some(Activation::Minimize(window.id))
        } else if self.pinned {
            Some(Activation::Launch(self.id.clone()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: WindowId, app_id: &str, focused: bool) -> Window {
        Window {
            id,
            app_id: app_id.into(),
            title: format!("Window {id}"),
            focused,
            minimized: false,
        }
    }
    fn apps() -> Vec<Application> {
        vec![Application {
            desktop_id: "nemo.desktop".into(),
            name: "Files".into(),
            icon: "folder".into(),
            startup_wm_class: Some("NemoWindow".into()),
        }]
    }

    #[test]
    fn pinned_windows_match_startup_class_and_keep_favorite_when_closed() {
        let mut taskbar = Taskbar::new(vec!["nemo".into(), "foot".into()]);
        taskbar.set_applications(apps());
        taskbar.upsert_window(window(1, "NemoWindow", true));
        let groups = taskbar.groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].id, "nemo.desktop");
        assert_eq!(groups[0].activate(), Some(Activation::Minimize(1)));
        taskbar.remove_window(1);
        assert_eq!(
            taskbar.groups()[0].activate(),
            Some(Activation::Launch("nemo.desktop".into()))
        );
    }

    #[test]
    fn groups_multiple_windows_and_focuses_an_unfocused_one() {
        let mut taskbar = Taskbar::new(vec![]);
        taskbar.set_applications(apps());
        taskbar.upsert_window(window(3, "nemo", true));
        taskbar.upsert_window(window(4, "NemoWindow", false));
        assert_eq!(taskbar.groups().len(), 1);
        assert_eq!(
            taskbar.groups()[0].activate(),
            Some(Activation::Activate(4))
        );
        taskbar.upsert_window(window(3, "other", false));
        assert_eq!(taskbar.groups().len(), 2);
        taskbar.remove_window(4);
        assert_eq!(taskbar.groups().len(), 1);
    }

    #[test]
    fn unknown_empty_ids_do_not_group_and_catalog_refresh_regroups() {
        let mut taskbar = Taskbar::new(vec!["nemo".into()]);
        taskbar.upsert_window(window(1, "NemoWindow", false));
        taskbar.upsert_window(window(2, "", false));
        taskbar.upsert_window(window(3, "", false));
        assert_eq!(taskbar.groups().len(), 4);
        taskbar.set_applications(apps());
        let groups = taskbar.groups();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].windows[0].id, 1);
        assert_eq!(groups[1].name, "Window 2");
        assert_eq!(groups[2].name, "Window 3");
    }
}
