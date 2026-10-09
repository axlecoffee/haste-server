// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
mod render;

use gloo_net::http::Request;
use leptos::{ev, prelude::*, task::spawn_local};
use serde::Deserialize;
use wasm_bindgen::JsValue;

const LANGUAGES: [(&str, &str); 17] = [
    ("txt", "Plain text"),
    ("md", "Markdown"),
    ("rs", "Rust"),
    ("ts", "TypeScript"),
    ("js", "JavaScript"),
    ("kt", "Kotlin"),
    ("py", "Python"),
    ("java", "Java"),
    ("json", "JSON"),
    ("sh", "Shell"),
    ("html", "HTML"),
    ("css", "CSS"),
    ("toml", "TOML"),
    ("yaml", "YAML"),
    ("go", "Go"),
    ("c", "C"),
    ("cpp", "C++"),
];

#[derive(Clone, Copy)]
struct Page {
    text: RwSignal<String>,
    id: RwSignal<String>,
    language: RwSignal<String>,
    editing: RwSignal<bool>,
    preview: RwSignal<bool>,
    busy: RwSignal<bool>,
    error: RwSignal<String>,
    generation: RwSignal<u32>,
}

#[derive(Deserialize)]
struct Document {
    data: String,
}

#[derive(Deserialize)]
struct Saved {
    key: String,
}

fn language_index(code: &str) -> usize {
    LANGUAGES
        .iter()
        .position(|(value, _)| *value == code)
        .unwrap_or(0)
}

fn language_name(code: &str) -> String {
    match LANGUAGES.iter().find(|(value, _)| *value == code) {
        Some((_, name)) => (*name).to_owned(),
        None => code.to_owned(),
    }
}

// #!/usr/bin/env python3 names the language outright
fn shebang(text: &str) -> Option<&'static str> {
    let line = text.lines().next()?.strip_prefix("#!")?;
    let mut words = line.split_whitespace();
    let mut interpreter = words.next()?;
    if interpreter.ends_with("/env") {
        interpreter = words.next()?;
    }
    let interpreter = interpreter.rsplit('/').next().unwrap_or(interpreter);
    match interpreter.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "python" => Some("py"),
        "node" => Some("js"),
        "bash" | "sh" | "zsh" | "dash" => Some("sh"),
        _ => None,
    }
}

// linguist names line up with the picker labels
fn code_for_name(name: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|(_, label)| *label == name)
        .map(|(value, _)| *value)
}

// linguist's .h rules are the one content-driven split it ships, the C family
fn linguist_header(text: &str) -> Option<&'static str> {
    if !text.contains("#include") {
        return None;
    }
    let hits = linguist::disambiguate("probe.h", text).ok()?;
    code_for_name(hits.first()?.name)
}

// linguist trusts extensions for unambiguous languages, so nameless pastes of
// those fall back to a keyword table, two hits or it stays plain
const MARKERS: [(&str, &[&str]); 13] = [
    ("md", &["```", "# ", "](", "## "]),
    (
        "cpp",
        &["std::", "cout", "template<", "namespace ", "public:"],
    ),
    (
        "c",
        &["#include", "int main", "printf(", "struct ", "malloc("],
    ),
    ("rs", &["fn ", "let ", "use ", "impl ", "println!"]),
    (
        "java",
        &["public ", "private ", "void ", "static ", "System."],
    ),
    ("kt", &["fun ", "val ", "override ", "object "]),
    ("py", &["def ", "import ", "elif ", "self", "print("]),
    ("ts", &["interface ", ": string", ": number", "export "]),
    ("js", &["function ", "const ", "=>", "console."]),
    ("go", &["func ", "package ", ":=", "fmt.", "defer "]),
    ("sh", &["if [", "$(", "echo ", "fi", "esac"]),
    ("html", &["</", "<div", "<html", "<body", "class="]),
    (
        "css",
        &["color:", "margin:", "padding:", "@media", "display:"],
    ),
];

fn markers(text: &str) -> Option<&'static str> {
    let mut best = None;
    let mut best_score = 1;
    for (code, words) in MARKERS {
        let score = words
            .iter()
            .copied()
            .filter(|word| text.contains(*word))
            .count();
        if score > best_score {
            best = Some(code);
            best_score = score;
        }
    }
    best
}

fn detect(text: &str) -> Option<&'static str> {
    if let Some(code) = shebang(text) {
        return Some(code);
    }
    if let Some(code) = linguist_header(text) {
        return Some(code);
    }
    markers(text)
}

// a real name gets linguist's take first, then the content guesses for itself
fn guess_language(id: &str, text: &str) -> Option<&'static str> {
    if let Ok(hits) = linguist::disambiguate(id, text)
        && let Some(code) = hits.first().and_then(|hit| code_for_name(hit.name))
    {
        return Some(code);
    }
    detect(text)
}

async fn fetch_document(id: &str) -> Result<String, &'static str> {
    let response = Request::get(&format!("/documents/{id}"))
        .send()
        .await
        .map_err(|_| "Could not connect to the server.")?;
    if !response.ok() {
        return Err(if response.status() == 404 {
            "Document not found."
        } else {
            "Could not load this document."
        });
    }
    let document = response
        .json::<Document>()
        .await
        .map_err(|_| "Invalid server response.")?;
    Ok(document.data)
}

async fn post_document(text: String) -> Result<String, &'static str> {
    let request = Request::post("/documents")
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(text)
        .map_err(|_| "Could not prepare this document.")?;
    let response = request
        .send()
        .await
        .map_err(|_| "Save failed. Your text is still in the editor.")?;
    if !response.ok() {
        return Err(if response.status() == 429 {
            "Upload limit reached. Wait a minute and try again."
        } else {
            "Save failed. Your text is still in the editor."
        });
    }
    let saved = response
        .json::<Saved>()
        .await
        .map_err(|_| "Invalid save response.")?;
    Ok(saved.key)
}

impl Page {
    fn navigate(self, path: &str) {
        let history = window()
            .history()
            .and_then(|history| history.push_state_with_url(&JsValue::NULL, "", Some(path)));
        if history.is_err() {
            self.error.set("Could not update browser history.".into());
            return;
        }
        self.load();
    }

    fn load(self) {
        self.generation.update(|n| *n += 1);
        let generation = self.generation.get_untracked();
        let path = window().location().pathname().unwrap_or_default();
        let id = path.trim_start_matches('/').to_owned();

        self.error.set(String::new());
        self.busy.set(false);
        self.text.set(String::new());
        self.id.set(id.clone());
        self.editing.set(id.is_empty());

        let extension = match id.split_once('.') {
            Some((_, extension)) => extension.to_owned(),
            None if matches!(id.as_str(), "about" | "readme") => "md".into(),
            None => "txt".into(),
        };
        let language = extension.to_ascii_lowercase();
        // reserved pages and known extensions speak for themselves
        let guess = match id.split_once('.') {
            Some(_) => !LANGUAGES.iter().any(|(value, _)| *value == language),
            None => !matches!(id.as_str(), "about" | "readme"),
        };
        self.preview
            .set(matches!(language.as_str(), "md" | "markdown"));
        self.language.set(language);

        if id.is_empty() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let result = fetch_document(&id).await;
            if self.generation.get_untracked() != generation {
                return;
            }
            match result {
                Ok(text) => {
                    if guess && let Some(code) = guess_language(&id, &text) {
                        self.language.set(code.to_owned());
                        self.preview.set(matches!(code, "md" | "markdown"));
                    }
                    self.text.set(text);
                }
                Err(message) => self.error.set(message.into()),
            }
            self.busy.set(false);
        });
    }

    fn save(self) {
        if !self.editing.get_untracked() || self.busy.get_untracked() {
            return;
        }
        let text = self.text.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        if text.len() > 400_000 {
            self.error
                .set("Maximum document size is 400000 UTF-8 bytes.".into());
            return;
        }
        let generation = self.generation.get_untracked();
        self.busy.set(true);
        self.error.set(String::new());
        spawn_local(async move {
            let result = post_document(text).await;
            if self.generation.get_untracked() != generation {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(key) => {
                    let extension = self.language.get_untracked();
                    let path = if extension == "txt" {
                        format!("/{key}")
                    } else {
                        format!("/{key}.{extension}")
                    };
                    self.navigate(&path);
                }
                Err(message) => self.error.set(message.into()),
            }
        });
    }

    fn duplicate(self) {
        if self.busy.get_untracked() {
            return;
        }
        let text = self.text.get_untracked();
        let language = self.language.get_untracked();
        self.navigate("/");
        self.text.set(text);
        self.language.set(language);
    }
}

// a flat listbox instead of <select>, the OS popup cannot be styled
#[component]
fn LanguagePicker(page: Page) -> impl IntoView {
    let open = RwSignal::new(false);
    let active = RwSignal::new(0usize);

    let select = move |value: &str| {
        page.language.set(value.to_owned());
        open.set(false);
    };

    // focus stays on the button while the list is open
    let keys = move |event: web_sys::KeyboardEvent| {
        let last = LANGUAGES.len() - 1;
        match event.key().as_str() {
            "ArrowDown" => {
                event.prevent_default();
                if open.get_untracked() {
                    active.update(|index| *index = (*index + 1).min(last));
                } else {
                    active.set(language_index(&page.language.get_untracked()));
                    open.set(true);
                }
            }
            "ArrowUp" => {
                event.prevent_default();
                if open.get_untracked() {
                    active.update(|index| *index = index.saturating_sub(1));
                } else {
                    active.set(language_index(&page.language.get_untracked()));
                    open.set(true);
                }
            }
            "Home" if open.get_untracked() => {
                event.prevent_default();
                active.set(0);
            }
            "End" if open.get_untracked() => {
                event.prevent_default();
                active.set(last);
            }
            "Enter" | " " if open.get_untracked() => {
                event.prevent_default();
                select(LANGUAGES[active.get_untracked()].0);
            }
            "Escape" if open.get_untracked() => {
                event.prevent_default();
                open.set(false);
            }
            _ => {}
        }
    };

    let option_list = move || {
        LANGUAGES.into_iter().enumerate().map(|(index, (value, name))| {
            view! {
                <li
                    id=format!("language-{index}")
                    role="option"
                    aria-selected=move || if page.language.get() == value { "true" } else { "false" }
                    class:active=move || active.get() == index
                    on:click=move |_| select(value)
                    on:mouseover=move |_| active.set(index)
                >
                    {name}
                </li>
            }
        }).collect_view()
    };

    view! {
        <div class="picker">
            <button
                type="button"
                class="language"
                aria-label=move || format!("Language, {}", language_name(&page.language.get()))
                aria-haspopup="listbox"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                aria-activedescendant=move || open.get().then(|| format!("language-{}", active.get()))
                on:click=move |_| {
                    if open.get_untracked() {
                        open.set(false);
                    } else {
                        active.set(language_index(&page.language.get_untracked()));
                        open.set(true);
                    }
                }
                on:blur=move |_| open.set(false)
                on:keydown=keys
            >
                {move || language_name(&page.language.get())}
                <span class="caret" aria-hidden="true"></span>
            </button>
            <Show when=move || open.get()>
                <ul role="listbox" aria-label="Language" on:mousedown=move |event| event.prevent_default()>
                    {option_list}
                </ul>
            </Show>
        </div>
    }
}

#[component]
fn App() -> impl IntoView {
    let page = Page {
        text: RwSignal::new(String::new()),
        id: RwSignal::new(String::new()),
        language: RwSignal::new("txt".into()),
        editing: RwSignal::new(true),
        preview: RwSignal::new(false),
        busy: RwSignal::new(false),
        error: RwSignal::new(String::new()),
        generation: RwSignal::new(0),
    };
    page.load();

    let history = window_event_listener(ev::popstate, move |_| page.load());
    let keyboard = window_event_listener(ev::keydown, move |event| {
        if !event.ctrl_key() && !event.meta_key() {
            return;
        }
        match event.key().to_ascii_lowercase().as_str() {
            "s" => {
                event.prevent_default();
                page.save();
            }
            "e" => {
                event.prevent_default();
                page.duplicate();
            }
            "n" if event.shift_key() => {
                event.prevent_default();
                page.navigate("/");
            }
            _ => {}
        }
    });
    on_cleanup(move || {
        history.remove();
        keyboard.remove();
    });

    let rendered = Memo::new(move |_| {
        if page.editing.get() {
            return String::new();
        }
        let text = page.text.get();
        if page.preview.get() && matches!(page.language.get().as_str(), "md" | "markdown") {
            render::markdown(&text)
        } else {
            render::code(&text, &page.language.get())
        }
    });
    let markdown = move || {
        !page.editing.get()
            && page.preview.get()
            && matches!(page.language.get().as_str(), "md" | "markdown")
    };
    let line_numbers = move || {
        let count = page.text.get().matches('\n').count() + 1;
        (1..=count)
            .map(|number| number.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };

    let editor = NodeRef::<leptos::html::Textarea>::new();
    let gutter = NodeRef::<leptos::html::Pre>::new();
    view! {
        <main aria-label="Paste editor">
            <header>
                <span class="status" role="status">{move || if page.busy.get() { "Loading..." } else { "" }}</span>
                <div class="options">
                    <LanguagePicker page=page />
                    <Show when=move || !page.editing.get() && matches!(page.language.get().as_str(), "md" | "markdown")>
                        <button class="preview" on:click=move |_| page.preview.update(|value| *value = !*value)>
                            {move || if page.preview.get() { "View source" } else { "Render Markdown" }}
                        </button>
                    </Show>
                </div>
            </header>
            <div class="notice" role="alert" hidden=move || page.error.get().is_empty()>
                {move || page.error.get()}
            </div>
            <div class="workspace" class:markdown=markdown class:editing=move || page.editing.get()>
                <pre node_ref=gutter class="gutter" aria-hidden="true" hidden=markdown>{line_numbers}</pre>
                <Show when=move || page.editing.get() fallback=move || view! {
                    <Show when=markdown fallback=move || view! {
                        <pre class="source"><code inner_html=move || rendered.get()></code></pre>
                    }>
                        <article class="rendered" inner_html=move || rendered.get()></article>
                    </Show>
                }>
                    <textarea node_ref=editor aria-label="Document text" spellcheck="false" autofocus
                        prop:value=move || page.text.get()
                        disabled=move || page.busy.get()
                        on:input=move |event| {
                            let text = event_target_value(&event);
                            // first content into an empty editor names itself when it can
                            if page.text.get_untracked().is_empty()
                                && page.language.get_untracked() == "txt"
                                && let Some(code) = detect(&text)
                            {
                                page.language.set(code.to_owned());
                            }
                            page.text.set(text);
                        }
                        on:scroll=move |_| {
                            if let (Some(input), Some(lines)) = (editor.get(), gutter.get()) {
                                lines.set_scroll_top(input.scroll_top());
                            }
                        }></textarea>
                </Show>
            </div>
            <footer>
                <nav aria-label="Information">
                    <a href="/about" on:click=move |event| {
                        event.prevent_default();
                        page.navigate("/about");
                    }>"about"</a>
                    <a href="https://axle.coffee" target="_blank" rel="noopener noreferrer">"axle.coffee"</a>
                </nav>
                <span class="size" aria-live="polite">{move || format!("{} bytes", page.text.get().len())}</span>
                <nav aria-label="Document actions">
                    <button class="save" title="Save (Ctrl/Cmd+S)"
                        disabled=move || !page.editing.get() || page.busy.get() || page.text.get().trim().is_empty()
                        on:click=move |_| page.save()>"Save"</button>
                    <button title="New (Ctrl/Cmd+Shift+N)" disabled=move || page.busy.get()
                        on:click=move |_| page.navigate("/")>"New"</button>
                    <button title="Duplicate & Edit (Ctrl/Cmd+E)"
                        disabled=move || page.editing.get() || page.busy.get()
                        on:click=move |_| page.duplicate()>"Duplicate & Edit"</button>
                    <a class="button"
                        class:disabled=move || page.editing.get() || page.busy.get()
                        aria-disabled=move || page.editing.get() || page.busy.get()
                        href=move || if page.editing.get() { "#".into() } else { format!("/raw/{}", page.id.get()) }
                        on:click=move |event| if page.editing.get() || page.busy.get() { event.prevent_default(); }>"Raw Text"</a>
                </nav>
            </footer>
        </main>
    }
}

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_languages() {
        assert_eq!(detect("#!/usr/bin/env python3\nprint(1)\n"), Some("py"));
        assert_eq!(
            detect("public class A {\n    private void b() {}\n}\n"),
            Some("java")
        );
        assert_eq!(
            detect("fn main() {\n    let mut x = 1;\n    println!(\"{x}\");\n}\n"),
            Some("rs")
        );
        assert_eq!(detect("just a note about class and public things\n"), None);
    }

    #[test]
    fn headers_disambiguate_through_linguist() {
        let cpp = "#include <iostream>\nint main() { std::cout << 1; }";
        // a .h name plus content settles the family
        assert_eq!(guess_language("code.h", cpp), Some("cpp"));
        assert_eq!(detect(cpp), Some("cpp"));
        assert_eq!(
            detect("#include <stdio.h>\nint main() { printf(\"x\"); }\n"),
            Some("c")
        );
    }
}
