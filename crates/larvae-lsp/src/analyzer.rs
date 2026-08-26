/*!
Luau's analysis frontend, behind the seam.

The C++ session lives in `shim/shim.cpp`; this module is the Rust half:
the FFI declarations, the safe wrapper that implements the server's
[`Analysis`] trait, and the resolver callbacks that answer the frontend's
require questions from Rust.

Resolution here covers what a plain Luau project writes: a relative path
against the requiring file (init aware), an `@self` prefix, and the
aliases of the nearest `.luaurc` walking up from the requiring file. A
spec that resolves to a directory answers its init file. Everything else
returns nothing, and the frontend reports an unknown require, which is
what it should say.
*/

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};

use crate::resolve::resolve_spec;

use larvae::lsp::analysis::{Analysis, AnalysisCompletion, AnalysisDiag, ModuleHooks};

#[repr(C)]
struct RawDiag {
    start: u32,
    end: u32,
    severity: u8,
    message: *const c_char,
}

#[repr(C)]
struct RawCompletion {
    label: *const c_char,
    kind: u8,
}

#[allow(non_camel_case_types)]
type larvae_resolve_fn = extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *const c_char;
#[allow(non_camel_case_types)]
type larvae_load_fn = extern "C" fn(*mut c_void, *const c_char) -> *const c_char;

unsafe extern "C" {
    fn larvae_session_new() -> *mut c_void;
    fn larvae_set_definitions(s: *mut c_void, name: *const c_char, source: *const c_char) -> i32;
    fn larvae_session_free(s: *mut c_void);
    fn larvae_set_resolver(
        s: *mut c_void,
        userdata: *mut c_void,
        resolve: larvae_resolve_fn,
        load: larvae_load_fn,
    );
    fn larvae_open(s: *mut c_void, path: *const c_char, text: *const c_char);
    fn larvae_invalidate(s: *mut c_void, path: *const c_char);
    fn larvae_check(s: *mut c_void, path: *const c_char, out: *mut RawDiag, cap: usize) -> usize;
    fn larvae_hover(s: *mut c_void, path: *const c_char, byte: u32) -> *const c_char;
    fn larvae_completions(
        s: *mut c_void,
        path: *const c_char,
        byte: u32,
        out: *mut RawCompletion,
        cap: usize,
    ) -> usize;
}

/*
The state the resolver callbacks read. It lives in a Box whose address is
the `userdata` the shim hands back, so the callbacks find it without any
global. The string buffers hold the last answers, per the shim contract:
valid until the next call on the same session.
*/
struct ResolverState {
    resolve_buffer: Option<CString>,
    load_buffer: Option<CString>,
    /// The worm hooks the server installs; consulted before default resolution
    hooks: Option<ModuleHooks>,
    /*
    The DataModel map of the project, for `@game`.

    Absent until the server loads a config that describes one. A project with
    no rojo project file and no `[requires.mounts]` has no DataModel, and
    `@game` then resolves to nothing, which is the true answer.
    */
    mounts: Option<larvae::requires::datamodel::MountTable>,
}

extern "C" fn resolve_cb(
    userdata: *mut c_void,
    from: *const c_char,
    spec: *const c_char,
) -> *const c_char {
    let state = unsafe { &mut *(userdata as *mut ResolverState) };
    let from = unsafe { CStr::from_ptr(from) }.to_string_lossy();
    let spec = unsafe { CStr::from_ptr(spec) }.to_string_lossy();

    /*
    The worms answer first. A worm that claims the spec gives the analyzer
    its module; every other spec falls through to default resolution, which
    is the hook-or-fallthrough shape the plan draws.
    */
    if let Some(hooks) = &state.hooks
        && let Some(path) = (hooks.resolve)(Path::new(from.as_ref()), &spec)
    {
        state.resolve_buffer = CString::new(path).ok();

        return state
            .resolve_buffer
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr());
    }

    match resolve_spec(Path::new(from.as_ref()), &spec, state.mounts.as_ref()) {
        Some(path) => {
            let text = path.to_string_lossy().into_owned();

            state.resolve_buffer = CString::new(text).ok();

            state
                .resolve_buffer
                .as_ref()
                .map_or(std::ptr::null(), |c| c.as_ptr())
        }

        None => std::ptr::null(),
    }
}

extern "C" fn load_cb(userdata: *mut c_void, path: *const c_char) -> *const c_char {
    let state = unsafe { &mut *(userdata as *mut ResolverState) };
    let path = unsafe { CStr::from_ptr(path) }.to_string_lossy();

    // A worm-resolved module loads through the worm, lowered to Luau.
    if let Some(hooks) = &state.hooks
        && let Some(text) = (hooks.load)(path.as_ref())
    {
        state.load_buffer = CString::new(text).ok();

        return state
            .load_buffer
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr());
    }

    match std::fs::read_to_string(path.as_ref()) {
        Ok(text) => {
            // A required file can hold larvae syntax; the analyzer reads stock Luau.
            let text = larvae::lsp::analysis::plain_view(&text).into_owned();

            state.load_buffer = CString::new(text).ok();

            state
                .load_buffer
                .as_ref()
                .map_or(std::ptr::null(), |c| c.as_ptr())
        }

        Err(_) => std::ptr::null(),
    }
}
pub struct LuauAnalysis {
    session: *mut c_void,
    /// Owned by the session for its lifetime; the shim only borrows it
    resolver: Box<ResolverState>,
    /// Path strings the session knows, so invalidate spells them the same way
    keys: HashMap<PathBuf, CString>,
    /// The service names, extracted from the definitions once
    services: Vec<String>,
}

// One session, used from the one server thread; the raw pointer is why
// the compiler cannot see it.
unsafe impl Send for LuauAnalysis {}

impl LuauAnalysis {
    pub fn new() -> Self {
        let mut resolver = Box::new(ResolverState {
            resolve_buffer: None,
            load_buffer: None,
            hooks: None,
            mounts: None,
        });

        let session = unsafe { larvae_session_new() };

        unsafe {
            larvae_set_resolver(
                session,
                &mut *resolver as *mut ResolverState as *mut c_void,
                resolve_cb,
                load_cb,
            );
        }

        let mut new = Self {
            session,
            resolver,
            keys: HashMap::new(),
            services: Vec::new(),
        };

        larvae::lsp::analysis::Analysis::definitions(&mut new, "@roblox", GLOBAL_TYPES);

        new
    }

    fn key(&mut self, path: &Path) -> *const c_char {
        self.keys
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                CString::new(path.to_string_lossy().into_owned()).unwrap_or_default()
            })
            .as_ptr()
    }
}

impl Drop for LuauAnalysis {
    fn drop(&mut self) {
        unsafe { larvae_session_free(self.session) };
    }
}

impl Analysis for LuauAnalysis {
    fn set_mounts(&mut self, mounts: larvae::requires::datamodel::MountTable) {
        self.resolver.mounts = Some(mounts);
    }

    fn services(&mut self) -> Vec<String> {
        if self.services.is_empty() {
            /*
            The first line of the definitions is machine metadata,
            `--#METADATA#{...}`, and its SERVICES array is the authority:
            luau-lsp writes it from the API dump for exactly this use.
            */
            self.services = GLOBAL_TYPES
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("--#METADATA#"))
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|meta| {
                    meta.get("SERVICES").and_then(|s| s.as_array()).map(|list| {
                        list.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                })
                .unwrap_or_default();

            self.services.sort();
            self.services.dedup();
        }

        self.services.clone()
    }

    fn set_module_hooks(&mut self, hooks: ModuleHooks) {
        self.resolver.hooks = Some(hooks);
    }

    fn definitions(&mut self, name: &str, source: &str) -> bool {
        let (Ok(name), Ok(source)) = (CString::new(name), CString::new(source)) else {
            return false;
        };

        unsafe { larvae_set_definitions(self.session, name.as_ptr(), source.as_ptr()) == 0 }
    }

    fn open(&mut self, path: &Path, text: &str) {
        let Ok(text) = CString::new(text) else {
            return;
        };

        let key = self.key(path);

        unsafe { larvae_open(self.session, key, text.as_ptr()) };
    }

    fn check(&mut self, path: &Path) -> Vec<AnalysisDiag> {
        let key = self.key(path);
        let mut raw: Vec<RawDiag> = Vec::with_capacity(256);
        let n = unsafe { larvae_check(self.session, key, raw.as_mut_ptr(), 256) };

        unsafe { raw.set_len(n) };

        raw.iter()
            .map(|d| AnalysisDiag {
                span: (d.start, d.end),
                severity: d.severity,
                message: unsafe { CStr::from_ptr(d.message) }
                    .to_string_lossy()
                    .into_owned(),
                code: None,
            })
            .collect()
    }

    fn hover(&mut self, path: &Path, at: u32) -> Option<String> {
        let key = self.key(path);
        let text = unsafe { larvae_hover(self.session, key, at) };

        if text.is_null() {
            return None;
        }

        Some(
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn completions(&mut self, path: &Path, at: u32) -> Vec<AnalysisCompletion> {
        let key = self.key(path);
        let mut raw: Vec<RawCompletion> = Vec::with_capacity(256);
        let n = unsafe { larvae_completions(self.session, key, at, raw.as_mut_ptr(), 256) };

        unsafe { raw.set_len(n) };

        raw.iter()
            .map(|c| AnalysisCompletion {
                label: unsafe { CStr::from_ptr(c.label) }
                    .to_string_lossy()
                    .into_owned(),
                kind: c.kind,
                detail: None,
            })
            .collect()
    }

    fn invalidate(&mut self, path: &Path) {
        let key = self.key(path);

        unsafe { larvae_invalidate(self.session, key) };
    }
}
