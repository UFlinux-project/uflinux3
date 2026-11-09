//! GIO owns desktop-entry parsing/visibility/Exec expansion. Workers return only
//! owned metadata, never GTK widgets or thread-bound GObjects.
use gio::prelude::*;

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub name_key: String,
    pub search_key: String,
}
#[derive(Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub favorites: Vec<Entry>,
}
pub fn fold(text: &str) -> String {
    glib::casefold(glib::normalize(text, glib::NormalizeMode::AllCompose)).to_string()
}
fn entry(info: &gio::AppInfo) -> Option<Entry> {
    if !info.should_show() {
        return None;
    }
    let id = info.id()?.to_string();
    let name = info.display_name().to_string();
    let description = info.description().unwrap_or_default().to_string();
    let mut search = format!("{name} {description}");
    if let Some(desktop) = info.downcast_ref::<gio::DesktopAppInfo>() {
        search.push_str(&format!(
            " {} {}",
            desktop.generic_name().unwrap_or_default(),
            desktop.keywords().join(" ")
        ));
    }
    Some(Entry {
        id,
        name_key: fold(&name),
        search_key: fold(&search),
        name,
        description,
        icon: info
            .icon()
            .and_then(|i| i.to_string())
            .unwrap_or_else(|| "application-x-executable".into())
            .to_string(),
    })
}
pub fn load() -> Catalog {
    let mut entries = gio::AppInfo::all()
        .iter()
        .filter_map(entry)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        glib::GString::from(&a.name)
            .as_gstr()
            .collate(&b.name)
            .then_with(|| a.id.cmp(&b.id))
    });
    entries.dedup_by(|a, b| a.id == b.id);
    let mut favorites = Vec::new();
    let files = gio::AppInfo::default_for_type("inode/directory", false)
        .or_else(|| gio::DesktopAppInfo::new("thunar.desktop").map(|a| a.upcast()));
    let terminal = gio::DesktopAppInfo::new("foot.desktop").map(|a| a.upcast());
    let settings = gio::DesktopAppInfo::new("org.uflinux.Settings.desktop").map(|a| a.upcast());
    let browser = gio::AppInfo::default_for_type("x-scheme-handler/http", false);
    for info in [files, terminal, settings, browser].into_iter().flatten() {
        if let Some(mut item) = entry(&info) {
            if favorites.iter().any(|a: &Entry| a.id == item.id) {
                continue;
            }
            // Familiar fixed role names; application metadata remains searchable.
            if item.id == "foot.desktop" {
                item.name = crate::widgets::tr("Terminal").to_string();
            }
            if item.id == "thunar.desktop" {
                item.name = crate::widgets::tr("Files").to_string();
            }
            favorites.push(item);
        }
    }
    Catalog { entries, favorites }
}
pub fn matches(entry: &Entry, query: &str) -> Option<u8> {
    let query = fold(query);
    let query = query.trim();
    if query.is_empty() {
        return Some(2);
    }
    if !query
        .split_whitespace()
        .all(|word| entry.search_key.contains(word))
    {
        return None;
    }
    Some(if entry.name_key == query {
        0
    } else if entry.name_key.starts_with(query) {
        1
    } else {
        2
    })
}
pub fn search(entries: &[Entry], query: &str) -> Vec<Entry> {
    let mut result = entries
        .iter()
        .filter_map(|entry| matches(entry, query).map(|rank| (rank, entry)))
        .collect::<Vec<_>>();
    result.sort_by(|(ar, a), (br, b)| {
        ar.cmp(br)
            .then_with(|| glib::GString::from(&a.name).as_gstr().collate(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    result.into_iter().map(|(_, e)| e.clone()).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn item(name: &str, keywords: &str) -> Entry {
        Entry {
            id: format!("{name}.desktop"),
            name: name.into(),
            icon: String::new(),
            description: keywords.into(),
            name_key: fold(name),
            search_key: fold(&format!("{name} {keywords}")),
        }
    }
    #[test]
    fn exact_prefix_then_keywords() {
        let data = vec![
            item("Other", "terminal"),
            item("Terminal tools", ""),
            item("Terminal", ""),
        ];
        assert_eq!(
            search(&data, "terminal")
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["Terminal", "Terminal tools", "Other"]
        );
    }
    #[test]
    fn unicode_and_multiple_tokens() {
        let data = vec![item("Éditeur Straße", "text documents")];
        assert_eq!(search(&data, "E\u{301}DITEUR STRASSE").len(), 1);
        assert_eq!(search(&data, "documents text").len(), 1);
        assert!(search(&data, "documents missing").is_empty());
    }
    #[test]
    fn empty_and_literal_shell_text() {
        assert_eq!(search(&[item("Files", "")], "  ").len(), 1);
        assert!(search(&[item("Files", "")], "$(touch /tmp/unsafe)").is_empty());
    }
}
