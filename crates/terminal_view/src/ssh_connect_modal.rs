use crate::TerminalView;
use editor::Editor;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    Styled, WeakEntity, Window,
};
use settings::Settings;
use terminal::{TerminalBuilder, connection::ssh::SshConfig, terminal_settings::TerminalSettings};
use ui::prelude::*;
use util::paths::PathStyle;
use workspace::{ModalView, Pane, Workspace};

pub struct SshConnectModal {
    workspace: WeakEntity<Workspace>,
    pane: Entity<Pane>,
    editor: Entity<Editor>,
    error: Option<SharedString>,
}

impl SshConnectModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        pane: Entity<Pane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("user@host[:port]", window, cx);
            editor
        });

        cx.subscribe_in(&editor, window, Self::on_editor_event)
            .detach();

        Self {
            workspace,
            pane,
            editor,
            error: None,
        }
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &editor::EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let editor::EditorEvent::BufferEdited { .. } = event {
            self.error = None;
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.editor.read(cx).text(cx);
        match parse_ssh_string(&input) {
            Ok(config) => {
                self.connect(config, window, cx);
                cx.emit(DismissEvent);
            }
            Err(err) => {
                self.error = Some(err.into());
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn connect(&self, config: SshConfig, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };

        let settings = TerminalSettings::get_global(cx);
        let cursor_shape = settings.cursor_shape;
        let alternate_scroll = settings.alternate_scroll;
        let max_scroll_history_lines = settings.max_scroll_history_lines;
        let path_style = PathStyle::local();
        let window_id = window.window_handle().window_id().as_u64();
        let pane = self.pane.clone();
        let weak_workspace = self.workspace.clone();

        let terminal_task = TerminalBuilder::new_with_ssh(
            config,
            cursor_shape,
            alternate_scroll,
            max_scroll_history_lines,
            window_id,
            cx,
            path_style,
        );

        cx.spawn_in(window, async move |_, cx| {
            let terminal_builder = match terminal_task.await {
                Ok(builder) => builder,
                Err(error) => {
                    log::error!("Failed to create SSH terminal: {}", error);
                    return;
                }
            };

            workspace
                .update_in(cx, |workspace, window, cx| {
                    let terminal_handle = cx.new(|cx| terminal_builder.subscribe(cx));
                    let terminal_view = Box::new(cx.new(|cx| {
                        TerminalView::new(
                            terminal_handle,
                            weak_workspace.clone(),
                            workspace.database_id(),
                            workspace.project().downgrade(),
                            window,
                            cx,
                        )
                    }));

                    pane.update(cx, |pane, cx| {
                        pane.add_item(terminal_view, true, true, None, window, cx);
                    });
                })
                .ok();
        })
        .detach();
    }
}

impl ModalView for SshConnectModal {}

impl EventEmitter<DismissEvent> for SshConnectModal {}

impl Focusable for SshConnectModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for SshConnectModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .key_context("SshConnectModal")
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .elevation_3(cx)
            .w_96()
            .overflow_hidden()
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.colors().border_variant)
                    .child(self.editor.clone()),
            )
            .child(
                h_flex()
                    .bg(theme.colors().editor_background)
                    .rounded_b_sm()
                    .w_full()
                    .p_2()
                    .gap_1()
                    .when_some(self.error.clone(), |this, err| {
                        this.child(Label::new(err).size(LabelSize::Small).color(Color::Error))
                    })
                    .when(self.error.is_none(), |this| {
                        this.child(
                            Label::new("Enter SSH connection string")
                                .color(Color::Muted)
                                .size(LabelSize::Small),
                        )
                    }),
            )
    }
}

fn parse_ssh_string(input: &str) -> Result<SshConfig, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Connection string required".into());
    }

    let (user_host, port) = if let Some((left, port_str)) = input.rsplit_once(':') {
        let port = port_str
            .parse::<u16>()
            .map_err(|_| "Invalid port number")?;
        (left, port)
    } else {
        (input, 22)
    };

    let (username, host) = user_host
        .split_once('@')
        .ok_or("Format: user@host[:port]")?;

    if username.is_empty() || host.is_empty() {
        return Err("Username and host required".into());
    }

    Ok(SshConfig::new(host, port).with_username(username))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_string_basic() {
        let config = parse_ssh_string("root@192.168.1.100").unwrap();
        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 22);
        assert_eq!(config.username, Some("root".to_string()));
    }

    #[test]
    fn test_parse_ssh_string_with_port() {
        let config = parse_ssh_string("admin@example.com:2222").unwrap();
        assert_eq!(config.host, "example.com");
        assert_eq!(config.port, 2222);
        assert_eq!(config.username, Some("admin".to_string()));
    }

    #[test]
    fn test_parse_ssh_string_empty() {
        let result = parse_ssh_string("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Connection string required");
    }

    #[test]
    fn test_parse_ssh_string_no_at() {
        let result = parse_ssh_string("hostname");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Format: user@host[:port]");
    }

    #[test]
    fn test_parse_ssh_string_invalid_port() {
        let result = parse_ssh_string("user@host:notaport");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid port number");
    }

    #[test]
    fn test_parse_ssh_string_empty_username() {
        let result = parse_ssh_string("@host");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Username and host required");
    }

    #[test]
    fn test_parse_ssh_string_empty_host() {
        let result = parse_ssh_string("user@");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Username and host required");
    }

    #[test]
    fn test_parse_ssh_string_whitespace() {
        let config = parse_ssh_string("  user@host  ").unwrap();
        assert_eq!(config.host, "host");
        assert_eq!(config.username, Some("user".to_string()));
    }
}
