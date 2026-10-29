use dioxus::prelude::*;
use dioxus_primitives::label::{self, LabelProps};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_label: &'static str = "dx-label";
}

#[component]
pub fn Label(props: LabelProps) -> Element {
    rsx! {
        label::Label {
            class: Styles::dx_label,
            html_for: props.html_for,
            attributes: props.attributes,
            {props.children}
        }
    }
}
