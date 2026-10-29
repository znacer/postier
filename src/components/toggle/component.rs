use dioxus::prelude::*;
use dioxus_primitives::toggle::{self, ToggleProps};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_toggle: &'static str = "dx-toggle";
}

#[component]
pub fn Toggle(props: ToggleProps) -> Element {
    rsx! {
        toggle::Toggle {
            class: Styles::dx_toggle,
            pressed: props.pressed,
            default_pressed: props.default_pressed,
            disabled: props.disabled,
            on_pressed_change: props.on_pressed_change,
            onmounted: props.onmounted,
            onfocus: props.onfocus,
            onkeydown: props.onkeydown,
            attributes: props.attributes,
            {props.children}
        }
    }
}
