use dioxus::prelude::*;
use dioxus_primitives::progress::{self, ProgressProps};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_progress: &'static str = "dx-progress";
    const dx_progress_indicator: &'static str = "dx-progress-indicator";
}

#[component]
pub fn Progress(props: ProgressProps) -> Element {
    rsx! {
        progress::Progress {
            class: Styles::dx_progress,
            value: props.value,
            max: props.max,
            attributes: props.attributes,
            progress::ProgressIndicator { class: Styles::dx_progress_indicator }
        }
    }
}
