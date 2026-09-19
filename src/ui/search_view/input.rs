//! 搜索输入区与放大镜绘制。

use iced::widget::text_input;
use iced::widget::{
    button, canvas, container, mouse_area, pick_list, row, space::Space, stack, text, Button,
};
use iced::{alignment, border, Background, Border, Color, Element, Length, Theme};

use super::super::font::name_font;
use super::super::theme::ThemeTokens;
use super::{enabled_plugin_samples, input_id, provider_meta, Message, State};
pub(super) fn search_row<'a>(state: &'a State, tokens: ThemeTokens) -> Element<'a, Message> {
    let magnifier = mouse_area(canvas(Magnifier(tokens)).width(20.0).height(20.0))
        .on_press(Message::DragWindow);

    let placeholder = search_placeholder(state);
    let text_color = tokens.text_primary;
    let muted = tokens.text_muted;
    let accent = tokens.accent;

    let input = text_input(&placeholder, &state.query)
        .id(input_id())
        .on_input(Message::QueryChanged)
        .size(16.0)
        .style(move |_t, _s| text_input::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            icon: muted,
            placeholder: muted,
            value: text_color,
            selection: Color { a: 0.25, ..accent },
        });

    // 左侧前缀区：放大镜与模式胶囊收拢于稳定 Row 内部，输入框在父 Row 中的层级索引恒定为 1，杜绝失焦
    let prefix = if let Some(act) = &state.provider_mode {
        let meta = provider_meta(&act.provider_id);
        let mode_chip = container(
            text(meta.chip_label)
                .size(11.5)
                .font(name_font())
                .color(tokens.accent),
        )
        .padding([3.0, 8.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(Color {
                a: 0.15,
                ..tokens.accent
            })),
            border: Border {
                color: Color {
                    a: 0.35,
                    ..tokens.accent
                },
                width: 1.0,
                radius: border::radius(6.0),
            },
            ..container::Style::default()
        });
        row![magnifier, mode_chip]
            .spacing(8.0)
            .align_y(alignment::Alignment::Center)
    } else if state.files_mode {
        let files_chip = button(
            text("📁 文件")
                .size(11.5)
                .font(name_font())
                .color(tokens.accent),
        )
        .padding([3.0, 8.0])
        .on_press(Message::ToggleFiles)
        .style(move |_t, _s| button::Style {
            background: Some(Background::Color(Color {
                a: 0.15,
                ..tokens.accent
            })),
            text_color: tokens.accent,
            border: Border {
                color: Color {
                    a: 0.35,
                    ..tokens.accent
                },
                width: 1.0,
                radius: border::radius(6.0),
            },
            ..button::Style::default()
        });
        row![magnifier, files_chip]
            .spacing(8.0)
            .align_y(alignment::Alignment::Center)
    } else {
        row![magnifier]
            .spacing(0.0)
            .align_y(alignment::Alignment::Center)
    };

    let mut row_content = row![prefix, input.width(Length::Fill)]
        .spacing(10.0)
        .align_y(alignment::Alignment::Center);

    if !state.query.is_empty() {
        let clear: Button<Message> = button(
            text("×")
                .size(18.0)
                .color(tokens.text_muted)
                .align_x(alignment::Alignment::Center),
        )
        .padding([4.0, 7.0])
        .on_press(Message::ClearQuery)
        .style(move |_t, status| button::Style {
            background: if status == button::Status::Hovered {
                Some(Background::Color(Color {
                    a: 0.10,
                    ..tokens.text_primary
                }))
            } else {
                None
            },
            text_color: if status == button::Status::Hovered {
                tokens.text_primary
            } else {
                tokens.text_muted
            },
            border: Border::default().rounded(6.0),
            ..button::Style::default()
        });
        row_content = row_content.push(clear);
    }

    if state.files_mode && state.provider_mode.is_none() {
        let filter = pick_list(
            crate::system::everything::FileFilter::ALL,
            Some(state.file_filter),
            Message::FileFilterChanged,
        )
        .width(112.0)
        .padding([5.0, 8.0])
        .text_size(12.0)
        .font(name_font())
        .style(move |_theme, _status| pick_list::Style {
            text_color: tokens.text_primary,
            placeholder_color: tokens.text_muted,
            handle_color: tokens.text_muted,
            background: Background::Color(tokens.bg_elevated),
            border: Border {
                color: tokens.border_window,
                width: 1.0,
                radius: border::radius(6.0),
            },
        });
        row_content = row_content.push(filter);
    }

    let bg_input = tokens.bg_input;
    let styled = container(row_content)
        .width(Length::Fill)
        .padding([14.0, 18.0])
        .style(move |_t| container::Style {
            background: Some(Background::Color(bg_input)),
            ..container::Style::default()
        });

    // 顶部 6px 拖拽条（叠在搜索条上缘）
    let drag_strip =
        mouse_area(Space::new().width(Length::Fill).height(6.0)).on_press(Message::DragWindow);

    stack![styled, drag_strip].into()
}

fn search_placeholder(state: &State) -> String {
    if let Some(act) = &state.provider_mode {
        provider_meta(&act.provider_id).placeholder.to_string()
    } else {
        let samples = enabled_plugin_samples(state);
        if samples.is_empty() {
            "搜索应用、网址，或直接输入关键词".to_string()
        } else {
            format!("搜索应用；或 {}", samples.join(" · "))
        }
    }
}

struct Magnifier(ThemeTokens);

impl canvas::Program<Message> for Magnifier {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ring = iced::Point::new(9.0, 9.0);
        frame.fill(&canvas::Path::circle(ring, 7.0), self.0.text_muted);
        frame.fill(&canvas::Path::circle(ring, 5.0), self.0.bg_input);
        let handle = canvas::Path::new(|p| {
            p.move_to(iced::Point::new(12.22, 13.78));
            p.line_to(iced::Point::new(16.62, 18.18));
            p.line_to(iced::Point::new(18.18, 16.62));
            p.line_to(iced::Point::new(13.78, 12.22));
            p.close();
        });
        frame.fill(&handle, self.0.text_muted);
        vec![frame.into_geometry()]
    }
}
