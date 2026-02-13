use editor::Editor;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Window,
};
use terminal::{
    AuthMethod, ProtocolConfig, SessionConfig, SessionNode, SessionStoreEntity,
    SshSessionConfig, TelnetSessionConfig,
};
use ui::{prelude::*, Button, ButtonStyle, Color, Label, LabelSize, h_flex, v_flex};
use uuid::Uuid;
use workspace::ModalView;

pub struct SessionEditModal {
    session_id: Uuid,
    session_store: Entity<SessionStoreEntity>,
    name_editor: Entity<Editor>,
    host_editor: Entity<Editor>,
    port_editor: Entity<Editor>,
    username_editor: Entity<Editor>,
    password_editor: Entity<Editor>,
    protocol: ProtocolType,
    focus_handle: FocusHandle,
}

#[derive(Clone, Copy, PartialEq)]
enum ProtocolType {
    Ssh,
    Telnet,
}

impl SessionEditModal {
    pub fn new(session_id: Uuid, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session_store = SessionStoreEntity::global(cx);
        let focus_handle = cx.focus_handle();

        let (name, host, port, username, password, protocol) = {
            let store = session_store.read(cx);
            if let Some(SessionNode::Session(session)) = store.store().find_node(session_id) {
                extract_session_data(session)
            } else {
                (String::new(), String::new(), 22, String::new(), String::new(), ProtocolType::Ssh)
            }
        };

        let name_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(name, window, cx);
            editor.set_placeholder_text("Session Name", window, cx);
            editor
        });

        let host_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(host, window, cx);
            editor.set_placeholder_text("Host", window, cx);
            editor
        });

        let port_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(port.to_string(), window, cx);
            editor.set_placeholder_text("Port", window, cx);
            editor
        });

        let username_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(username, window, cx);
            editor.set_placeholder_text("Username", window, cx);
            editor
        });

        let password_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(password, window, cx);
            editor.set_placeholder_text("Password", window, cx);
            editor
        });

        Self {
            session_id,
            session_store,
            name_editor,
            host_editor,
            port_editor,
            username_editor,
            password_editor,
            protocol,
            focus_handle,
        }
    }

    fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_editor.read(cx).text(cx);
        let host = self.host_editor.read(cx).text(cx);
        let port = self
            .port_editor
            .read(cx)
            .text(cx)
            .parse::<u16>()
            .unwrap_or(if self.protocol == ProtocolType::Ssh { 22 } else { 23 });
        let username = self.username_editor.read(cx).text(cx);
        let password = self.password_editor.read(cx).text(cx);

        let protocol = self.protocol;
        self.session_store.update(cx, |store, cx| {
            store.update_session(
                self.session_id,
                |session| {
                    session.name = name;
                    match protocol {
                        ProtocolType::Ssh => {
                            session.protocol = ProtocolConfig::Ssh(SshSessionConfig {
                                host,
                                port,
                                username: if username.is_empty() {
                                    None
                                } else {
                                    Some(username)
                                },
                                auth: if password.is_empty() {
                                    AuthMethod::Interactive
                                } else {
                                    AuthMethod::Password { password }
                                },
                                env: std::collections::HashMap::new(),
                                keepalive_interval_secs: Some(30),
                                initial_command: None,
                            });
                        }
                        ProtocolType::Telnet => {
                            session.protocol = ProtocolConfig::Telnet(TelnetSessionConfig {
                                host,
                                port,
                                username: if username.is_empty() {
                                    None
                                } else {
                                    Some(username)
                                },
                                password: if password.is_empty() {
                                    None
                                } else {
                                    Some(password)
                                },
                                encoding: None,
                            });
                        }
                    }
                },
                cx,
            );
        });

        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }
}

fn extract_session_data(session: &SessionConfig) -> (String, String, u16, String, String, ProtocolType) {
    match &session.protocol {
        ProtocolConfig::Ssh(ssh) => {
            let password = match &ssh.auth {
                AuthMethod::Password { password } => password.clone(),
                _ => String::new(),
            };
            (
                session.name.clone(),
                ssh.host.clone(),
                ssh.port,
                ssh.username.clone().unwrap_or_default(),
                password,
                ProtocolType::Ssh,
            )
        }
        ProtocolConfig::Telnet(telnet) => (
            session.name.clone(),
            telnet.host.clone(),
            telnet.port,
            telnet.username.clone().unwrap_or_default(),
            telnet.password.clone().unwrap_or_default(),
            ProtocolType::Telnet,
        ),
    }
}

impl ModalView for SessionEditModal {}

impl EventEmitter<DismissEvent> for SessionEditModal {}

impl Focusable for SessionEditModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SessionEditModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let border_color = theme.colors().border;
        let border_variant_color = theme.colors().border_variant;

        let protocol_label = match self.protocol {
            ProtocolType::Ssh => "SSH",
            ProtocolType::Telnet => "Telnet",
        };

        v_flex()
            .key_context("SessionEditModal")
            .track_focus(&self.focus_handle)
            .elevation_3(cx)
            .w_80()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .p_2()
                    .border_b_1()
                    .border_color(border_variant_color)
                    .justify_between()
                    .child(Label::new(format!("Edit {} Session", protocol_label)))
                    .child(
                        Button::new("close", "")
                            .icon(IconName::Close)
                            .icon_size(IconSize::Small)
                            .style(ButtonStyle::Transparent)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(Label::new("Name").size(LabelSize::Small).color(Color::Muted))
                            .child(
                                div()
                                    .w_full()
                                    .border_1()
                                    .border_color(border_color)
                                    .rounded_sm()
                                    .px_1()
                                    .py_px()
                                    .child(self.name_editor.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        Label::new("Host")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .border_1()
                                            .border_color(border_color)
                                            .rounded_sm()
                                            .px_1()
                                            .py_px()
                                            .child(self.host_editor.clone()),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_16()
                                    .gap_1()
                                    .child(
                                        Label::new("Port")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .border_1()
                                            .border_color(border_color)
                                            .rounded_sm()
                                            .px_1()
                                            .py_px()
                                            .child(self.port_editor.clone()),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        Label::new("Username")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .border_1()
                                            .border_color(border_color)
                                            .rounded_sm()
                                            .px_1()
                                            .py_px()
                                            .child(self.username_editor.clone()),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        Label::new("Password")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .border_1()
                                            .border_color(border_color)
                                            .rounded_sm()
                                            .px_1()
                                            .py_px()
                                            .child(self.password_editor.clone()),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .justify_end()
                    .border_t_1()
                    .border_color(border_variant_color)
                    .child(
                        Button::new("cancel", "Cancel")
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("save", "Save")
                            .style(ButtonStyle::Filled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save(window, cx);
                            })),
                    ),
            )
    }
}
