use std::{
	cell::{Cell, RefCell},
	ffi::OsString,
	os::windows::ffi::OsStringExt,
	path::PathBuf,
	ptr,
	rc::Rc,
};

use tauri::{AppHandle, Emitter, EventTarget, Manager, PhysicalPosition};
use tauri_plugin_fs::FsExt;
use windows::{
	Win32::{
		Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, POINT, POINTL, WPARAM},
		Graphics::Gdi::ScreenToClient,
		System::{
			Com::{
				DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL,
			},
			Memory::{GlobalLock, GlobalSize, GlobalUnlock},
			Ole::{
				CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY,
				DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, RegisterDragDrop,
				ReleaseStgMedium, RevokeDragDrop,
			},
			SystemServices::MODIFIERKEYS_FLAGS,
		},
		UI::{
			Shell::{
				DefSubclassProc, DragQueryFileW, HDROP, RemoveWindowSubclass,
				SetWindowSubclass,
			},
			WindowsAndMessaging::{EnumChildWindows, IsChild, WM_DESTROY},
		},
	},
	core::{BOOL, Ref, Result},
};
use windows_core::implement;

const CARD_PREFIX: &str = "authlib-injector:yggdrasil-server:";
const MAX_CARD_LENGTH: usize = 8192;

pub fn install(app: &AppHandle) {
	let Some(main) = app.get_webview_window("main") else {
		return;
	};
	let app = app.clone();
	if let Err(error) = main.with_webview(move |webview| {
		let mut container = HWND::default();
		let parent = unsafe { webview.controller().ParentWindow(&mut container) };
		if let Err(error) = parent {
			tracing::warn!("Failed to locate the WebView2 drop host: {error}");
			return;
		}
		register(container, app);
	}) {
		tracing::warn!("Failed to set up WebView2 drop handling: {error}");
	}
}

type Registrations = Rc<RefCell<Vec<HWND>>>;

fn register(container: HWND, app: AppHandle) {
	unsafe extern "system" fn collect_child(child: HWND, data: LPARAM) -> BOOL {
		// SAFETY: EnumChildWindows calls this synchronously with the live vector below.
		unsafe { &mut *(data.0 as *mut Vec<HWND>) }.push(child);
		true.into()
	}

	unsafe extern "system" fn cleanup(
		hwnd: HWND,
		message: u32,
		wparam: WPARAM,
		lparam: LPARAM,
		id: usize,
		data: usize,
	) -> LRESULT {
		if message == WM_DESTROY {
			// SAFETY: the subclass owns this Rc reference until the host is destroyed.
			let _ = unsafe { RemoveWindowSubclass(hwnd, Some(cleanup), id) };
			let registrations =
				unsafe { Rc::from_raw(data as *const RefCell<Vec<HWND>>) };
			let targets = std::mem::take(&mut *registrations.borrow_mut());
			for target in targets {
				if target == hwnd || unsafe { IsChild(hwnd, target).as_bool() } {
					let _ = unsafe { RevokeDragDrop(target) };
				}
			}
		}
		unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
	}

	let mut children = Vec::<HWND>::new();
	// Wry can register targets on descendants owned by the browser process.
	// Cleanup on the host window keeps OLE calls on the registering thread.
	let _ = unsafe {
		EnumChildWindows(
			Some(container),
			Some(collect_child),
			LPARAM(ptr::from_mut(&mut children) as isize),
		)
	};
	let registrations = Registrations::default();
	let state = Rc::into_raw(registrations.clone());
	let subclass = unsafe {
		SetWindowSubclass(container, Some(cleanup), 0, state as usize)
	};
	if !subclass.as_bool() {
		unsafe { drop(Rc::from_raw(state)) };
		tracing::warn!("Failed to attach WebView2 drop cleanup");
		return;
	}

	let target: IDropTarget = DropTarget {
		container,
		app,
		active: Cell::new(None),
	}
	.into();
	if let Err(error) = unsafe { RegisterDragDrop(container, &target) } {
		tracing::warn!("Failed to register the WebView2 drop host: {error}");
		return;
	}
	registrations.borrow_mut().push(container);
	for child in children {
		// Replace Wry's existing targets. The container covers unregistered children.
		if unsafe { RevokeDragDrop(child) }.is_ok() {
			match unsafe { RegisterDragDrop(child, &target) } {
				Ok(()) => registrations.borrow_mut().push(child),
				Err(error) => tracing::warn!(
					"Failed to register a WebView2 child drop target: {error}"
				),
			}
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DropKind {
	Files,
	AuthCard,
}

enum DropData {
	Files(Vec<PathBuf>),
	AuthCard(String),
}

impl DropData {
	fn read(data: &IDataObject) -> Option<Self> {
		file_paths(data)
			.map(Self::Files)
			.or_else(|| auth_card_text(data).map(Self::AuthCard))
	}

	fn kind(&self) -> DropKind {
		match self {
			Self::Files(_) => DropKind::Files,
			Self::AuthCard(_) => DropKind::AuthCard,
		}
	}
}

#[derive(Clone, serde::Serialize)]
struct FileDrop<'a> {
	#[serde(skip_serializing_if = "Option::is_none")]
	paths: Option<&'a [PathBuf]>,
	position: PhysicalPosition<i32>,
}

#[derive(Clone, serde::Serialize)]
struct AuthCardDrop {
	text: String,
	position: PhysicalPosition<i32>,
}

#[implement(IDropTarget)]
struct DropTarget {
	container: HWND,
	app: AppHandle,
	active: Cell<Option<DropKind>>,
}

impl DropTarget {
	fn position(&self, screen: &POINTL) -> PhysicalPosition<i32> {
		let mut point = POINT {
			x: screen.x,
			y: screen.y,
		};
		let _ = unsafe { ScreenToClient(self.container, &mut point) };
		PhysicalPosition::new(point.x, point.y)
	}

	fn emit(&self, name: &str, payload: impl serde::Serialize + Clone) {
		if let Err(error) =
			self.app.emit_to(EventTarget::labeled("main"), name, payload)
		{
			tracing::warn!("Failed to emit drop event {name}: {error}");
		}
	}

	fn emit_files(
		&self,
		name: &str,
		paths: Option<&[PathBuf]>,
		point: &POINTL,
	) {
		self.emit(
			name,
			FileDrop {
				paths,
				position: self.position(point),
			},
		);
	}

	fn allow_paths(&self, paths: &[PathBuf]) {
		// Match Tauri's window drop handling and the fs plugin's drop hook.
		let assets = self.app.asset_protocol_scope();
		let files = self.app.fs_scope();
		for path in paths {
			if path.is_file() {
				let _ = assets.allow_file(path);
				let _ = files.allow_file(path);
			} else {
				let _ = assets.allow_directory(path, true);
				let _ = files.allow_directory(path, true);
			}
		}
	}
}

fn set_copy_effect(effect: *mut DROPEFFECT, accepted: bool) -> bool {
	// SAFETY: the non-null effect pointer is the current OLE callback's in/out parameter.
	let Some(effect) = (unsafe { effect.as_mut() }) else {
		return false;
	};
	let copy = accepted && effect.contains(DROPEFFECT_COPY);
	*effect = if copy { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
	copy
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
	fn DragEnter(
		&self,
		data: Ref<'_, IDataObject>,
		_: MODIFIERKEYS_FLAGS,
		point: &POINTL,
		effect: *mut DROPEFFECT,
	) -> Result<()> {
		let data = data.as_ref().and_then(DropData::read);
		let kind = data.as_ref().map(DropData::kind);
		self.active.set(kind);
		set_copy_effect(effect, kind.is_some());
		if let Some(DropData::Files(paths)) = data {
			self.emit_files("tauri://drag-enter", Some(&paths), point);
		}
		Ok(())
	}

	fn DragOver(
		&self,
		_: MODIFIERKEYS_FLAGS,
		point: &POINTL,
		effect: *mut DROPEFFECT,
	) -> Result<()> {
		let kind = self.active.get();
		if kind == Some(DropKind::Files) {
			self.emit_files("tauri://drag-over", None, point);
		}
		set_copy_effect(effect, kind.is_some());
		Ok(())
	}

	fn DragLeave(&self) -> Result<()> {
		if self.active.take() == Some(DropKind::Files) {
			self.emit("tauri://drag-leave", ());
		}
		Ok(())
	}

	fn Drop(
		&self,
		data: Ref<'_, IDataObject>,
		_: MODIFIERKEYS_FLAGS,
		point: &POINTL,
		effect: *mut DROPEFFECT,
	) -> Result<()> {
		let kind = self.active.take();
		let data = if set_copy_effect(effect, kind.is_some()) {
			data.as_ref()
				.and_then(DropData::read)
				.filter(|data| kind == Some(data.kind()))
		} else {
			None
		};
		match data {
			Some(DropData::Files(paths)) => {
				self.allow_paths(&paths);
				self.emit_files("tauri://drag-drop", Some(&paths), point);
			}
			Some(DropData::AuthCard(text)) => self.emit(
				"acl-auth-card-drop",
				AuthCardDrop {
					text,
					position: self.position(point),
				},
			),
			None => {
				set_copy_effect(effect, false);
				if kind == Some(DropKind::Files) {
					self.emit("tauri://drag-leave", ());
				}
			}
		}
		Ok(())
	}
}

struct Medium(STGMEDIUM);

impl Medium {
	fn get(data: &IDataObject, format: u16) -> Option<Self> {
		unsafe { data.GetData(&global_format(format)) }.ok().map(Self)
	}

	fn global(&self) -> Option<HGLOBAL> {
		if self.0.tymed != TYMED_HGLOBAL.0 as u32 {
			return None;
		}
		let handle = unsafe { self.0.u.hGlobal };
		(!handle.is_invalid()).then_some(handle)
	}
}

fn global_format(format: u16) -> FORMATETC {
	FORMATETC {
		cfFormat: format,
		dwAspect: DVASPECT_CONTENT.0,
		lindex: -1,
		tymed: TYMED_HGLOBAL.0 as u32,
		..Default::default()
	}
}

impl Drop for Medium {
	fn drop(&mut self) {
		// GetData transfers ownership of the medium to the caller.
		unsafe { ReleaseStgMedium(&mut self.0) };
	}
}

fn file_paths(data: &IDataObject) -> Option<Vec<PathBuf>> {
	let medium = Medium::get(data, CF_HDROP.0)?;
	let files = HDROP(medium.global()?.0);
	let count = unsafe { DragQueryFileW(files, u32::MAX, None) };
	let paths: Vec<_> = (0..count)
		.filter_map(|index| {
			let length =
				unsafe { DragQueryFileW(files, index, None) } as usize;
			let mut name = vec![0; length + 1];
			let copied = unsafe { DragQueryFileW(files, index, Some(&mut name)) }
				as usize;
			(copied > 0 && copied <= length)
				.then(|| OsString::from_wide(&name[..copied]).into())
		})
		.collect();
	(!paths.is_empty()).then_some(paths)
}

fn auth_card_text(data: &IDataObject) -> Option<String> {
	let medium = Medium::get(data, CF_UNICODETEXT.0)?;
	let memory = medium.global()?;
	let count = (unsafe { GlobalSize(memory) } / size_of::<u16>())
		.min(MAX_CARD_LENGTH + 1);
	let text = unsafe { GlobalLock(memory) }.cast::<u16>();
	if text.is_null() {
		return None;
	}
	// SAFETY: the allocation stays locked for the read and count cannot exceed its size.
	let text = read_auth_card(unsafe { std::slice::from_raw_parts(text, count) });
	let _ = unsafe { GlobalUnlock(memory) };
	text
}

fn read_auth_card(buffer: &[u16]) -> Option<String> {
	let end = buffer
		.iter()
		.take(MAX_CARD_LENGTH + 1)
		.position(|unit| *unit == 0)?;
	let text = String::from_utf16(&buffer[..end]).ok()?;
	let card = text.trim();
	let encoded = card.strip_prefix(CARD_PREFIX)?;
	if encoded.bytes().enumerate().any(|(index, byte)| {
		byte == b'%'
			&& !encoded
				.as_bytes()
				.get(index + 1..index + 3)
				.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
	}) {
		return None;
	}
	let decoded = urlencoding::decode(encoded).ok()?;
	if decoded.contains(char::is_control)
		|| !decoded
			.get(..8)
			.is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
	{
		return None;
	}
	let url = url::Url::parse(&decoded).ok()?;
	(url.host().is_some()
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.query().is_none()
		&& url.fragment().is_none()
		&& url.as_str().len() <= 2048)
		.then(|| card.to_owned())
}

#[cfg(test)]
mod tests {
	use super::*;
	use windows::Win32::{
		System::{
			Com::STGMEDIUM_0,
			Memory::{GHND, GlobalAlloc},
			Ole::{OleInitialize, OleUninitialize},
		},
		UI::Shell::{DROPFILES, SHCreateDataObject},
	};

	fn card(encoded: &str) -> String {
		format!("{CARD_PREFIX}{encoded}")
	}

	fn unicode(text: &str) -> Vec<u16> {
		text.encode_utf16().chain([0]).collect()
	}

	fn set_data(
		data: &IDataObject,
		format: u16,
		header: Option<DROPFILES>,
		text: &[u16],
	) {
		let offset = header.map_or(0, |_| size_of::<DROPFILES>());
		let size = offset + size_of_val(text);
		let memory = unsafe { GlobalAlloc(GHND, size) }.unwrap();
		let medium = Medium(STGMEDIUM {
			tymed: TYMED_HGLOBAL.0 as u32,
			u: STGMEDIUM_0 { hGlobal: memory },
			..Default::default()
		});
		let buffer = unsafe { GlobalLock(memory) }.cast::<u8>();
		assert!(!buffer.is_null());
		unsafe {
			if let Some(header) = header {
				buffer.cast::<DROPFILES>().write(header);
			}
			ptr::copy_nonoverlapping(
				text.as_ptr(),
				buffer.add(offset).cast(),
				text.len(),
			);
			let _ = GlobalUnlock(memory);
			data.SetData(&global_format(format), &medium.0, true).unwrap();
		}
		std::mem::forget(medium);
	}

	#[test]
	fn reads_an_authlib_card_without_decoding_the_event_payload() {
		let text = card("https%3A%2F%2Fexample.test%2Fyggdrasil");
		assert_eq!(read_auth_card(&unicode(&text)), Some(text));
	}

	#[test]
	fn rejects_unrelated_malformed_or_oversized_drop_text() {
		for text in [
			"https://example.test/".into(),
			card("http%3A%2F%2Fexample.test"),
			card("https%3A%2F%2Fexample.test%2F%ZZ"),
			card("https%3A%2F%2Fexample.test%2F%0A"),
			card("https%3A%2F%2Fuser%3Apassword%40example.test"),
			card("https%3A%2F%2Fexample.test%3Fquery"),
			card("https%3A%2F%2Fexample.test%23fragment"),
			card(&format!(
				"https%3A%2F%2Fexample.test%2F{}",
				"a".repeat(MAX_CARD_LENGTH)
			)),
		] {
			assert_eq!(read_auth_card(&unicode(&text)), None);
		}
		assert_eq!(read_auth_card(&[u16::from(b'a'); MAX_CARD_LENGTH]), None);
	}

	#[test]
	fn native_file_and_directory_drop_take_precedence_over_auth_card() {
		struct Ole;
		impl Drop for Ole {
			fn drop(&mut self) {
				unsafe { OleUninitialize() };
			}
		}
		unsafe { OleInitialize(None) }.unwrap();
		let _ole = Ole;
		let data: IDataObject =
			unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }
				.unwrap();
		let text = card("https%3A%2F%2Fexample.test%2Fyggdrasil");
		set_data(&data, CF_UNICODETEXT.0, None, &unicode(&text));
		assert!(matches!(
			DropData::read(&data),
			Some(DropData::AuthCard(found)) if found == text
		));

		let paths = [r"C:\fixture\a.mrpack", r"C:\fixture\pack-directory"];
		let names = paths
			.into_iter()
			.flat_map(unicode)
			.chain([0])
			.collect::<Vec<_>>();
		let header = DROPFILES {
			pFiles: size_of::<DROPFILES>() as u32,
			fWide: true.into(),
			..Default::default()
		};
		set_data(&data, CF_HDROP.0, Some(header), &names);
		assert!(matches!(
			DropData::read(&data),
			Some(DropData::Files(files)) if files == paths.map(PathBuf::from)
		));
	}
}
