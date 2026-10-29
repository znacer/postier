use dioxus::prelude::*;
use dioxus_primitives::radio_group::{self, RadioGroupProps, RadioItemProps};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_radio_group: &'static str = "dx-radio-group";
    const dx_radio_item: &'static str = "dx-radio-item";
}

#[component]
pub fn RadioGroup(props: RadioGroupProps) -> Element {
    rsx! {
        radio_group::RadioGroup {
            class: Styles::dx_radio_group,
            value: props.value,
            default_value: props.default_value,
            on_value_change: props.on_value_change,
            disabled: props.disabled,
            required: props.required,
            name: props.name,
            horizontal: props.horizontal,
            roving_loop: props.roving_loop,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn RadioItem(props: RadioItemProps) -> Element {
    rsx! {
        radio_group::RadioItem {
            class: Styles::dx_radio_item.to_string(),
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            attributes: props.attributes,
            {props.children}
        }
    }
}
