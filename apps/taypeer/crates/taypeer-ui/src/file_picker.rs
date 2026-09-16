//! OS file selection boundary. Selecting a path never performs file I/O.
use gpui_kit::{App, Global, PathPromptOptions, Result};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
};

type Selection<T> = Pin<Box<dyn Future<Output = Result<Option<T>>>>>;

trait FilePicker {
    fn open(&self, cx: &App, options: PathPromptOptions) -> Selection<Vec<PathBuf>>;
    fn save(&self, cx: &App, directory: &Path, suggested: Option<&str>) -> Selection<PathBuf>;
}

/// Application-global file chooser capability, replaceable by a scripted fixture.
pub struct FileDialogs(Rc<dyn FilePicker>);
impl Global for FileDialogs {}
impl FileDialogs {
    /// Use the GPUI platform file chooser.
    pub fn native() -> Self {
        Self(Rc::new(Native))
    }
}
struct Native;
impl FilePicker for Native {
    fn open(&self, cx: &App, options: PathPromptOptions) -> Selection<Vec<PathBuf>> {
        let response = cx.prompt_for_paths(options);
        Box::pin(async move { response.await? })
    }
    fn save(&self, cx: &App, directory: &Path, suggested: Option<&str>) -> Selection<PathBuf> {
        let response = cx.prompt_for_new_path(directory, suggested);
        Box::pin(async move { response.await? })
    }
}
/// Prompt for existing paths; cancellation returns None without performing file I/O.
pub fn open(cx: &App, options: PathPromptOptions) -> Selection<Vec<PathBuf>> {
    cx.global::<FileDialogs>().0.open(cx, options)
}
/// Prompt for a destination; selecting it does not write the file.
pub fn save(cx: &App, directory: &Path, suggested: Option<&str>) -> Selection<PathBuf> {
    cx.global::<FileDialogs>().0.save(cx, directory, suggested)
}

#[cfg(feature = "ui-test-support")]
/// Scripted file chooser for isolated UI scenarios.
pub mod fixture {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};
    enum Answer {
        Open(Option<Vec<PathBuf>>),
        Save(Option<PathBuf>),
    }
    #[derive(Clone, Default)]
    /// Ordered dialog answers; shared clones consume the same queue.
    pub struct ScriptedPicker(Rc<RefCell<VecDeque<Answer>>>);
    impl ScriptedPicker {
        /// Install this queue as the application file chooser.
        pub fn install(&self, cx: &mut App) {
            cx.set_global(FileDialogs(Rc::new(self.clone())));
        }
        /// Enqueue the next Open response; None simulates cancellation.
        pub fn open(&self, paths: Option<Vec<PathBuf>>) {
            self.0.borrow_mut().push_back(Answer::Open(paths));
        }
        /// Enqueue the next Save response; None simulates cancellation.
        pub fn save(&self, path: Option<PathBuf>) {
            self.0.borrow_mut().push_back(Answer::Save(path));
        }
        /// Assert that all expected chooser interactions occurred.
        pub fn assert_consumed(&self) {
            assert!(
                self.0.borrow().is_empty(),
                "expected file dialog was not requested"
            );
        }
    }
    impl FilePicker for ScriptedPicker {
        fn open(&self, _: &App, _: PathPromptOptions) -> Selection<Vec<PathBuf>> {
            let Some(Answer::Open(paths)) = self.0.borrow_mut().pop_front() else {
                panic!("unexpected Open dialog; enqueue an explicit response");
            };
            Box::pin(async move { Ok(paths) })
        }
        fn save(&self, _: &App, _: &Path, _: Option<&str>) -> Selection<PathBuf> {
            let Some(Answer::Save(path)) = self.0.borrow_mut().pop_front() else {
                panic!("unexpected Save dialog; enqueue an explicit response");
            };
            Box::pin(async move { Ok(path) })
        }
    }
}
