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

pub(super) struct FileDialogs(Rc<dyn FilePicker>);
impl Global for FileDialogs {}
impl FileDialogs {
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
pub(super) fn open(cx: &App, options: PathPromptOptions) -> Selection<Vec<PathBuf>> {
    cx.global::<FileDialogs>().0.open(cx, options)
}
pub(super) fn save(cx: &App, directory: &Path, suggested: Option<&str>) -> Selection<PathBuf> {
    cx.global::<FileDialogs>().0.save(cx, directory, suggested)
}

#[cfg(feature = "ui-test-support")]
pub(super) mod fixture {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};
    enum Answer {
        Open(Option<Vec<PathBuf>>),
        Save(Option<PathBuf>),
    }
    #[derive(Clone, Default)]
    pub(crate) struct ScriptedPicker(Rc<RefCell<VecDeque<Answer>>>);
    impl ScriptedPicker {
        pub fn install(&self, cx: &mut App) {
            cx.set_global(FileDialogs(Rc::new(self.clone())));
        }
        pub fn open(&self, paths: Option<Vec<PathBuf>>) {
            self.0.borrow_mut().push_back(Answer::Open(paths));
        }
        pub fn save(&self, path: Option<PathBuf>) {
            self.0.borrow_mut().push_back(Answer::Save(path));
        }
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
