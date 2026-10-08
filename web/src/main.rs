// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
mod render;

use gloo_net::http::Request;
use leptos::{ev, prelude::*, task::spawn_local};
use serde::Deserialize;

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
struct Document { data: String }
#[derive(Deserialize)]
struct Saved { key: String }

impl Page {
	fn navigate(self, path: &str) {
		if window().history().and_then(|h| h.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))).is_err() {
			self.error.set("Could not update browser history.".into());
			return;
		}
		self.load();
	}

	fn load(self) {
		self.generation.update(|n| *n += 1);
		let generation = self.generation.get_untracked();
		let id = window().location().pathname().unwrap_or_default().trim_start_matches('/').to_owned();
		self.error.set(String::new());
		self.busy.set(false);
		self.text.set(String::new());
		self.id.set(id.clone());
		self.editing.set(id.is_empty());
		let language = id.split_once('.').map(|(_, ext)| ext).unwrap_or(if matches!(id.as_str(), "about" | "readme") { "md" } else { "txt" }).to_ascii_lowercase();
		self.preview.set(matches!(language.as_str(), "md" | "markdown"));
		self.language.set(language);
		if id.is_empty() { return; }
		self.busy.set(true);
		spawn_local(async move {
			let result = async {
				let response = Request::get(&format!("/documents/{id}")).send().await.map_err(|_| "Could not connect to the server.")?;
				if !response.ok() { return Err(if response.status() == 404 { "Document not found." } else { "Could not load this document." }); }
				response.json::<Document>().await.map_err(|_| "Invalid server response.")
			}.await;
			if self.generation.get_untracked() != generation { return; }
			match result { Ok(doc) => self.text.set(doc.data), Err(e) => self.error.set(e.into()) }
			self.busy.set(false);
		});
	}

	fn save(self) {
		if !self.editing.get_untracked() || self.busy.get_untracked() || self.text.get_untracked().trim().is_empty() { return; }
		let text = self.text.get_untracked();
		if text.len() > 400_000 {
			self.error.set("Maximum document size is 400000 UTF-8 bytes.".into());
			return;
		}
		let generation = self.generation.get_untracked();
		self.busy.set(true);
		self.error.set(String::new());
		spawn_local(async move {
			let result = async {
				let response = Request::post("/documents").header("Content-Type", "text/plain; charset=utf-8").body(text)
					.map_err(|_| "Could not prepare this document.")?.send().await.map_err(|_| "Save failed. Your text is still in the editor.")?;
				if !response.ok() { return Err(if response.status() == 429 { "Upload limit reached. Wait a minute and try again." } else { "Save failed. Your text is still in the editor." }); }
				response.json::<Saved>().await.map_err(|_| "Invalid save response.")
			}.await;
			if self.generation.get_untracked() != generation { return; }
			self.busy.set(false);
			match result {
				Ok(saved) => {
					let ext = self.language.get_untracked();
					self.navigate(&format!("/{}{}", saved.key, if ext == "txt" { String::new() } else { format!(".{ext}") }));
				}
				Err(e) => self.error.set(e.into()),
			}
		});
	}

	fn duplicate(self) {
		if self.busy.get_untracked() { return; }
		let text = self.text.get_untracked();
		let language = self.language.get_untracked();
		self.navigate("/");
		self.text.set(text);
		self.language.set(language);
	}
}

#[component]
fn App() -> impl IntoView {
	let page = Page {
		text: RwSignal::new(String::new()), id: RwSignal::new(String::new()),
		language: RwSignal::new("txt".into()), editing: RwSignal::new(true),
		preview: RwSignal::new(false), busy: RwSignal::new(false),
		error: RwSignal::new(String::new()), generation: RwSignal::new(0),
	};
	page.load();
	let history = window_event_listener(ev::popstate, move |_| page.load());
	let keyboard = window_event_listener(ev::keydown, move |event| {
		if event.ctrl_key() || event.meta_key() {
			match event.key().to_ascii_lowercase().as_str() {
				"s" => { event.prevent_default(); page.save(); }
				"e" => { event.prevent_default(); page.duplicate(); }
				"n" if event.shift_key() => { event.prevent_default(); page.navigate("/"); }
				_ => {}
			}
		}
	});
	on_cleanup(move || { history.remove(); keyboard.remove(); });
	let rendered = Memo::new(move |_| {
		if page.editing.get() { return String::new(); }
		let text = page.text.get();
		if page.preview.get() && matches!(page.language.get().as_str(), "md" | "markdown") {
			render::markdown(&text)
		} else { render::code(&text, &page.language.get()) }
	});
	let numbers = move || (1..=page.text.get().bytes().filter(|b| *b == b'\n').count() + 1).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
	let markdown = move || !page.editing.get() && page.preview.get() && matches!(page.language.get().as_str(), "md" | "markdown");
	let editor = NodeRef::<leptos::html::Textarea>::new();
	let gutter = NodeRef::<leptos::html::Pre>::new();
	view! {
		<main aria-label="Paste editor">
			<header>
				<span class="status" role="status">{move || if page.busy.get() { "Loading..." } else { "" }}</span>
				<div class="options">
					<select id="language" aria-label="Language" prop:value=move || page.language.get() on:change=move |event| {
						page.language.set(event_target_value(&event));
					}>
						<option value="txt">"Plain text"</option><option value="md">"Markdown"</option>
						<option value="rs">"Rust"</option><option value="ts">"TypeScript"</option>
						<option value="js">"JavaScript"</option><option value="kt">"Kotlin"</option>
						<option value="py">"Python"</option><option value="java">"Java"</option>
						<option value="json">"JSON"</option><option value="sh">"Shell"</option>
						<option value="html">"HTML"</option><option value="css">"CSS"</option>
						<option value="toml">"TOML"</option><option value="yaml">"YAML"</option>
						<option value="go">"Go"</option><option value="c">"C"</option><option value="cpp">"C++"</option>
					</select>
					<Show when=move || !page.editing.get() && matches!(page.language.get().as_str(), "md" | "markdown")>
						<button class="preview" on:click=move |_| page.preview.update(|value| *value = !*value)>{move || if page.preview.get() { "View source" } else { "Render Markdown" }}</button>
					</Show>
				</div>
			</header>
			<div class="notice" role="alert" hidden=move || page.error.get().is_empty()>{move || page.error.get()}</div>
			<div class="workspace" class:markdown=markdown class:editing=move || page.editing.get()>
				<pre node_ref=gutter class="gutter" aria-hidden="true" hidden=markdown>{numbers}</pre>
				<Show when=move || page.editing.get() fallback=move || view! {
					<Show when=markdown fallback=move || view! { <pre class="source"><code inner_html=move || rendered.get()></code></pre> }>
						<article class="rendered" inner_html=move || rendered.get()></article>
					</Show>
				}>
					<textarea node_ref=editor aria-label="Document text" spellcheck="false" autofocus
						prop:value=move || page.text.get() disabled=move || page.busy.get()
						on:input=move |event| page.text.set(event_target_value(&event))
						on:scroll=move |_| {
							if let (Some(input), Some(lines)) = (editor.get(), gutter.get()) { lines.set_scroll_top(input.scroll_top()); }
						}></textarea>
				</Show>
			</div>
			<footer>
				<nav aria-label="Information"><a href="/about" on:click=move |event| { event.prevent_default(); page.navigate("/about"); }>"about"</a><a href="https://axle.coffee" target="_blank" rel="noopener noreferrer">"axle.coffee"</a></nav>
				<span class="size" aria-live="polite">{move || format!("{} bytes", page.text.get().len())}</span>
				<nav aria-label="Document actions">
					<button class="save" title="Save (Ctrl/Cmd+S)" disabled=move || !page.editing.get() || page.busy.get() || page.text.get().trim().is_empty() on:click=move |_| page.save()>"Save"</button>
					<button title="New (Ctrl/Cmd+Shift+N)" disabled=move || page.busy.get() on:click=move |_| page.navigate("/")>"New"</button>
					<button title="Duplicate & Edit (Ctrl/Cmd+E)" disabled=move || page.editing.get() || page.busy.get() on:click=move |_| page.duplicate()>"Duplicate & Edit"</button>
					<a class="button" class:disabled=move || page.editing.get() || page.busy.get() aria-disabled=move || page.editing.get() || page.busy.get()
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