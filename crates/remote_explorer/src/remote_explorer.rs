mod quick_add;
mod session_edit_modal;

use std::ops::Range;

use anyhow::Result;
use gpui::{
    Action, App, AppContext as _, AsyncWindowContext, ClickEvent, Context, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, MouseDownEvent, ParentElement, Point, Render, Styled,
    Subscription, UniformListScrollHandle, WeakEntity, Window, anchored, deferred, px, uniform_list,
};
use terminal::{ProtocolConfig, SessionNode, SessionStoreEntity, SessionStoreEvent};
use ui::{
    prelude::*, Color, ContextMenu, Disclosure, Icon, IconName, IconSize, Label, LabelSize,
    ListItem, ListItemSpacing, h_flex, v_flex,
};
use uuid::Uuid;
use workspace::{
    Pane, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};
use zed_actions::remote_explorer::ToggleFocus;

pub use quick_add::*;
pub use session_edit_modal::SessionEditModal;

const REMOTE_EXPLORER_PANEL_KEY: &str = "RemoteExplorerPanel";

pub fn init(cx: &mut App) {
    SessionStoreEntity::init(cx);

    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<RemoteExplorer>(window, cx);
        });
    })
    .detach();
}

/// A flattened tree entry for uniform list rendering.
#[derive(Clone, Debug)]
pub struct FlattenedEntry {
    pub id: Uuid,
    pub depth: usize,
    pub node: SessionNode,
}

pub struct RemoteExplorer {
    session_store: Entity<SessionStoreEntity>,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
    visible_entries: Vec<FlattenedEntry>,
    workspace: WeakEntity<Workspace>,
    width: Option<Pixels>,
    quick_add_expanded: bool,
    quick_add_area: QuickAddArea,
    selected_entry_id: Option<Uuid>,
    context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}

impl RemoteExplorer {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| Self::new(workspace, window, cx))
        })
    }

    pub fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session_store = SessionStoreEntity::global(cx);
        let focus_handle = cx.focus_handle();
        let weak_workspace = workspace.weak_handle();

        let session_store_subscription =
            cx.subscribe(&session_store, |this, _, event, cx| match event {
                SessionStoreEvent::Changed
                | SessionStoreEvent::SessionAdded(_)
                | SessionStoreEvent::SessionRemoved(_)
                | SessionStoreEvent::CredentialPresetChanged => {
                    this.update_visible_entries(cx);
                }
            });

        let quick_add_area =
            QuickAddArea::new(session_store.clone(), weak_workspace.clone(), window, cx);

        let username_editor = quick_add_area.telnet_section.username_editor.clone();
        let password_editor = quick_add_area.telnet_section.password_editor.clone();

        let username_subscription =
            cx.subscribe(&username_editor, |this, _, event: &editor::EditorEvent, cx| {
                if matches!(event, editor::EditorEvent::BufferEdited { .. }) {
                    this.quick_add_area.telnet_section.clear_preset_selection();
                    cx.notify();
                }
            });

        let password_subscription =
            cx.subscribe(&password_editor, |this, _, event: &editor::EditorEvent, cx| {
                if matches!(event, editor::EditorEvent::BufferEdited { .. }) {
                    this.quick_add_area.telnet_section.clear_preset_selection();
                    cx.notify();
                }
            });

        let mut this = Self {
            session_store,
            focus_handle,
            scroll_handle: UniformListScrollHandle::new(),
            visible_entries: Vec::new(),
            workspace: weak_workspace,
            width: None,
            quick_add_expanded: true,
            quick_add_area,
            selected_entry_id: None,
            context_menu: None,
            _subscriptions: vec![
                session_store_subscription,
                username_subscription,
                password_subscription,
            ],
        };

        this.update_visible_entries(cx);
        this
    }

    fn update_visible_entries(&mut self, cx: &mut Context<Self>) {
        let session_store = self.session_store.read(cx);
        let store = session_store.store();

        let mut entries = Vec::new();
        Self::flatten_nodes(&store.root, 0, &mut entries);
        self.visible_entries = entries;
        cx.notify();
    }

    fn flatten_nodes(nodes: &[SessionNode], depth: usize, result: &mut Vec<FlattenedEntry>) {
        for node in nodes {
            result.push(FlattenedEntry {
                id: node.id(),
                depth,
                node: node.clone(),
            });

            if let SessionNode::Group(group) = node {
                if group.expanded {
                    Self::flatten_nodes(&group.children, depth + 1, result);
                }
            }
        }
    }

    fn toggle_expanded(&mut self, id: Uuid, _window: &mut Window, cx: &mut Context<Self>) {
        self.session_store.update(cx, |store, cx| {
            store.toggle_group_expanded(id, cx);
        });
        self.update_visible_entries(cx);
    }

    fn toggle_quick_add(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.quick_add_expanded = !self.quick_add_expanded;
        cx.notify();
    }

    fn select_entry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.selected_entry_id = Some(id);
        cx.notify();
    }

    fn connect_session(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let session_store = self.session_store.read(cx);
        let Some(node) = session_store.store().find_node(id) else {
            return;
        };

        let SessionNode::Session(session) = node else {
            return;
        };

        match &session.protocol {
            ProtocolConfig::Ssh(ssh_config) => {
                let workspace = self.workspace.clone();
                let pane = self.get_terminal_pane(cx);
                if let (Some(workspace), Some(pane)) = (workspace.upgrade(), pane) {
                    connect_ssh(ssh_config.clone(), workspace, pane, window, cx);
                }
            }
            ProtocolConfig::Telnet(telnet_config) => {
                log::info!(
                    "Telnet connection not yet implemented: {}:{}",
                    telnet_config.host,
                    telnet_config.port
                );
            }
        }
    }

    fn deploy_context_menu(
        &mut self,
        position: Point<Pixels>,
        entry_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session_store = self.session_store.read(cx);
        let Some(node) = session_store.store().find_node(entry_id) else {
            return;
        };

        let SessionNode::Session(_session) = node else {
            return;
        };

        let workspace = self.workspace.clone();
        let session_store_entity = self.session_store.clone();

        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            let workspace_for_edit = workspace.clone();

            menu.entry("Edit Session", None, move |window, cx| {
                if let Some(workspace) = workspace_for_edit.upgrade() {
                    workspace.update(cx, |ws, cx| {
                        ws.toggle_modal(window, cx, |window, cx| {
                            SessionEditModal::new(entry_id, window, cx)
                        });
                    });
                }
            })
            .entry("Delete Session", None, move |_window, cx| {
                session_store_entity.update(cx, |store, cx| {
                    store.remove_node(entry_id, cx);
                });
            })
        });

        window.focus(&context_menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&context_menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu.take();
            cx.notify();
        });
        self.context_menu = Some((context_menu, position, subscription));
        cx.notify();
    }

    fn get_terminal_pane(&self, cx: &App) -> Option<Entity<Pane>> {
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);

        if let Some(terminal_panel) =
            workspace.panel::<terminal_view::terminal_panel::TerminalPanel>(cx)
        {
            terminal_panel.read(cx).pane()
        } else {
            None
        }
    }

    fn handle_auto_recognize_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let pane = self.get_terminal_pane(cx);
        if let Some((ssh_config, workspace, pane)) = self
            .quick_add_area
            .handle_auto_recognize_confirm(workspace, pane, window, cx)
        {
            connect_ssh(ssh_config, workspace, pane, window, cx);
        }
    }

    fn handle_telnet_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let pane = self.get_terminal_pane(cx);
        self.quick_add_area
            .handle_telnet_connect(workspace, pane, window, cx);
    }

    fn handle_ssh_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let pane = self.get_terminal_pane(cx);
        if let Some((ssh_config, workspace, pane)) = self
            .quick_add_area
            .handle_ssh_connect(workspace, pane, window, cx)
        {
            connect_ssh(ssh_config, workspace, pane, window, cx);
        }
    }

    fn render_quick_add_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let expanded = self.quick_add_expanded;

        h_flex()
            .id("quick-add-header")
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .cursor_pointer()
            .hover(|style| style.bg(theme.colors().ghost_element_hover))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_quick_add(window, cx);
            }))
            .child(Disclosure::new("quick-add-disclosure", expanded))
            .child(
                Label::new("Quick Add")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
    }

    fn render_quick_add_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .px_2()
            .pb_2()
            .gap_3()
            .child(self.render_auto_recognize_section(window, cx))
            .child(self.render_telnet_section(window, cx))
            .child(self.render_ssh_section(window, cx))
    }

    fn render_auto_recognize_section(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let editor = self.quick_add_area.auto_recognize.editor().clone();

        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::MagnifyingGlass)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Auto-recognize")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(theme.colors().border)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .on_action(cx.listener(|this, _: &menu::Confirm, window, cx| {
                                this.handle_auto_recognize_confirm(window, cx);
                            }))
                            .child(editor),
                    ),
            )
            .child(
                Label::new("Supports: IP, IP:port, IP user pass")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }

    fn render_telnet_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let session_store = self.session_store.read(cx);
        let presets = session_store.credential_presets().to_vec();
        let has_presets = !presets.is_empty();
        let preset_label = self.quick_add_area.telnet_section.get_preset_label(cx);

        let ip_editor = self.quick_add_area.telnet_section.ip_editor.clone();
        let port_editor = self.quick_add_area.telnet_section.port_editor.clone();
        let username_editor = self.quick_add_area.telnet_section.username_editor.clone();
        let password_editor = self.quick_add_area.telnet_section.password_editor.clone();

        let preset_menu = if has_presets {
            Some(ui::ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
                menu = menu.entry("Custom", None, |_window, _cx| {});
                for preset in &presets {
                    let name = preset.name.clone();
                    menu = menu.entry(name, None, |_window, _cx| {});
                }
                menu
            }))
        } else {
            None
        };

        let theme = cx.theme();
        let border_color = theme.colors().border;

        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Terminal)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("Telnet Quick Connect")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(border_color)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .child(ip_editor),
                    )
                    .child(
                        div()
                            .w_16()
                            .border_1()
                            .border_color(border_color)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .child(port_editor),
                    ),
            )
            .when_some(preset_menu, |this, menu| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(
                            Label::new("Preset:")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            ui::DropdownMenu::new("telnet-preset", preset_label, menu)
                                .trigger_size(ui::ButtonSize::Compact),
                        ),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(border_color)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .child(username_editor),
                    )
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(border_color)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .child(password_editor),
                    ),
            )
            .child(
                h_flex().w_full().justify_end().child(
                    ui::Button::new("telnet-connect", "Connect")
                        .style(ui::ButtonStyle::Filled)
                        .size(ui::ButtonSize::Compact)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.handle_telnet_connect(window, cx);
                        })),
                ),
            )
    }

    fn render_ssh_section(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let host_editor = self.quick_add_area.ssh_section.editor().clone();

        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Server)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new("SSH Quick Connect")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .border_1()
                            .border_color(theme.colors().border)
                            .rounded_sm()
                            .px_1()
                            .py_px()
                            .on_action(cx.listener(|this, _: &menu::Confirm, window, cx| {
                                this.handle_ssh_connect(window, cx);
                            }))
                            .child(host_editor),
                    )
                    .child(
                        ui::Button::new("ssh-connect", "Connect")
                            .style(ui::ButtonStyle::Filled)
                            .size(ui::ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_ssh_connect(window, cx);
                            })),
                    ),
            )
            .child(
                Label::new("Default: root/root")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }

    fn render_entry(&self, index: usize, _window: &mut Window, cx: &mut Context<Self>) -> ListItem {
        let entry = &self.visible_entries[index];
        let id = entry.id;
        let depth = entry.depth;
        let is_selected = self.selected_entry_id == Some(id);

        let (icon, name, is_group, is_expanded) = match &entry.node {
            SessionNode::Group(group) => (
                if group.expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                },
                group.name.clone(),
                true,
                Some(group.expanded),
            ),
            SessionNode::Session(session) => (IconName::Server, session.name.clone(), false, None),
        };

        ListItem::new(id)
            .indent_level(depth)
            .indent_step_size(px(12.))
            .spacing(ListItemSpacing::Dense)
            .toggle(is_expanded)
            .toggle_state(is_selected)
            .when(is_group, |this| {
                this.on_toggle(cx.listener(move |this, _, window, cx| {
                    this.toggle_expanded(id, window, cx);
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.toggle_expanded(id, window, cx);
                }))
            })
            .when(!is_group, |this| {
                this.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.click_count() == 2 {
                        this.connect_session(id, window, cx);
                    } else {
                        this.select_entry(id, cx);
                    }
                }))
                .on_secondary_mouse_down(cx.listener(
                    move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.select_entry(id, cx);
                        this.deploy_context_menu(event.position, id, window, cx);
                    },
                ))
            })
            .start_slot(
                Icon::new(icon)
                    .color(Color::Muted)
                    .size(IconSize::Small),
            )
            .child(Label::new(name))
    }

    fn render_entries(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ListItem> {
        let mut items = Vec::with_capacity(range.len());
        for ix in range {
            items.push(self.render_entry(ix, window, cx));
        }
        items
    }
}

impl EventEmitter<PanelEvent> for RemoteExplorer {}

impl Render for RemoteExplorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let item_count = self.visible_entries.len();
        let quick_add_expanded = self.quick_add_expanded;

        v_flex()
            .id("remote-explorer")
            .size_full()
            .track_focus(&self.focus_handle(cx))
            .child(
                v_flex()
                    .w_full()
                    .border_b_1()
                    .border_color(theme.colors().border_variant)
                    .child(self.render_quick_add_header(cx))
                    .when(quick_add_expanded, |this| {
                        this.child(self.render_quick_add_content(window, cx))
                    }),
            )
            .child(if item_count > 0 {
                uniform_list(
                    "remote-explorer-list",
                    item_count,
                    cx.processor(|this, range: Range<usize>, window, cx| {
                        this.render_entries(range, window, cx)
                    }),
                )
                .flex_1()
                .track_scroll(&self.scroll_handle)
                .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .p_4()
                    .gap_2()
                    .child(Label::new("No saved sessions").color(Color::Muted))
                    .into_any_element()
            })
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(gpui::Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

impl Focusable for RemoteExplorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for RemoteExplorer {
    fn persistent_name() -> &'static str {
        "Remote Explorer"
    }

    fn panel_key() -> &'static str {
        REMOTE_EXPLORER_PANEL_KEY
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Left
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        _position: DockPosition,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn size(&self, _window: &Window, _cx: &App) -> Pixels {
        self.width.unwrap_or(px(240.))
    }

    fn set_size(&mut self, size: Option<Pixels>, _window: &mut Window, cx: &mut Context<Self>) {
        self.width = size;
        cx.notify();
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::Server)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Remote Explorer")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        10
    }
}
