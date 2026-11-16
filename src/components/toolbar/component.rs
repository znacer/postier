use dioxus::prelude::*;
use dioxus_primitives::toolbar::{self, ToolbarButtonProps, ToolbarProps, ToolbarSeparatorProps};
use dioxus_primitives::{dioxus_attributes::attributes, merge_attributes};

#[allow(non_upper_case_globals)]
struct Styles;
#[allow(non_upper_case_globals)]
impl Styles {
    const dx_toolbar: &'static str = "dx-toolbar";
    const dx_toolbar_separator: &'static str = "dx-toolbar-separator";
    const dx_toolbar_group: &'static str = "dx-toolbar-group";
}

#[component]
pub fn Toolbar(props: ToolbarProps) -> Element {
    let base = attributes!(div {
        class: Styles::dx_toolbar,
    });
    let merged = merge_attributes(vec![base, props.attributes]);

    rsx! {
        toolbar::Toolbar {
            aria_label: props.aria_label,
            disabled: props.disabled,
            horizontal: props.horizontal,
            attributes: merged,
            {props.children}
        }
    }
}

#[component]
pub fn ToolbarButton(props: ToolbarButtonProps) -> Element {
    rsx! {
        toolbar::ToolbarButton {
            index: props.index,
            disabled: props.disabled,
            on_click: props.on_click,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ToolbarSeparator(props: ToolbarSeparatorProps) -> Element {
    let base = attributes!(div {
        class: Styles::dx_toolbar_separator,
    });
    let merged = merge_attributes(vec![base, props.attributes]);

    rsx! {
        toolbar::ToolbarSeparator {
            decorative: props.decorative,
            horizontal: props.horizontal,
            attributes: merged,
        }
    }
}

#[component]
pub fn ToolbarGroup(
    #[props(extends = GlobalAttributes)]
    #[props(extends = div)]
    attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    let base = attributes!(div {
        class: Styles::dx_toolbar_group,
    });
    let merged = merge_attributes(vec![base, attributes]);

    rsx! {
        div { ..merged,{children} }
    }
}
