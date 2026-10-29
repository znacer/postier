use dioxus::prelude::*;
use dioxus_primitives::{dioxus_attributes::attributes, merge_attributes};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_skeleton: &'static str = "dx-skeleton";
}

#[component]
pub fn Skeleton(#[props(extends=GlobalAttributes)] attributes: Vec<Attribute>) -> Element {
    let base = attributes!(div {
        class: Styles::dx_skeleton,
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged }
    }
}
