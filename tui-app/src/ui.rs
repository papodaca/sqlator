use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};

use crate::app::{App, FocusPane, ItemKind};

pub fn draw_connection_list(f: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    let title = Paragraph::new(Line::from(vec![
        Span::styled(" SQLator", Style::default().fg(Color::Cyan).bold()),
        Span::raw(" \u{2014} Select Connection"),
    ]))
    .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(title, chunks[0]);

    if app.connections.is_empty() {
        let empty = Paragraph::new(
            "No connections found.\n\nCreate connections using the SQLator desktop app.",
        )
        .block(Block::default().padding(ratatui::widgets::Padding::new(2, 2, 1, 1)))
        .style(Style::default().fg(Color::Yellow));
        f.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = app
            .connections
            .iter()
            .map(|conn| {
                let db_type_color = match conn.db_type.as_str() {
                    "postgres" | "postgresql" => Color::Blue,
                    "mysql" => Color::Magenta,
                    "sqlite" => Color::Green,
                    _ => Color::White,
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {} ", conn.db_type),
                        Style::default().fg(Color::Black).bg(db_type_color),
                    ),
                    Span::raw(" "),
                    Span::styled(&conn.name, Style::default().fg(Color::White)),
                    Span::styled(
                        format!("  {}@{}:{}", conn.username, conn.host, conn.port),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().padding(ratatui::widgets::Padding::new(1, 1, 0, 0)))
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("\u{25B6} ");

        let mut state = app.conn_list_state.clone();
        f.render_stateful_widget(list, chunks[1], &mut state);
    }

    if let Some(ref err) = app.connect_error {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" Error: ", Style::default().fg(Color::White).bg(Color::Red)),
            Span::styled(err.clone(), Style::default().fg(Color::Red)),
        ]));
        f.render_widget(footer, chunks[2]);
    } else {
        let footer = Paragraph::new(Line::from(vec![
            Span::styled(" \u{2191}\u{2193}", Style::default().fg(Color::Cyan)),
            Span::raw(" Navigate  "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" Connect  "),
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" Quit"),
        ]));
        f.render_widget(footer, chunks[2]);
    }
}

pub fn draw_connecting(f: &mut ratatui::Frame, app: &App) {
    let conn_name = app
        .active_connection
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("...");

    let spinner = app.spinner_char();
    let text = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {} ", spinner), Style::default().fg(Color::Cyan)),
        Span::raw(format!("Connecting to {}...", conn_name)),
    ]))
    .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(text, f.area());
}

pub fn draw_workspace(f: &mut ratatui::Frame, app: &App) {
    let conn_name = app
        .active_connection
        .as_ref()
        .map(|c| c.name.as_str())
        .unwrap_or("");

    let conn_detail = app
        .active_connection
        .as_ref()
        .map(|c| format!("{}@{}/{}", c.username, c.host, c.database))
        .unwrap_or_default();

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(" SQLator", Style::default().fg(Color::Cyan).bold()),
        Span::raw(" \u{2014} "),
        Span::styled(conn_name, Style::default().fg(Color::White).bold()),
        Span::raw(" \u{2014} "),
        Span::styled(conn_detail, Style::default().fg(Color::DarkGray)),
    ]));
    f.render_widget(header, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(outer[1]);

    draw_schema_pane(f, app, body[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(body[1]);

    draw_editor_pane(f, app, right[0]);
    draw_results_pane(f, app, right[1]);

    let footer = draw_footer(app);
    f.render_widget(footer, outer[2]);
}

fn draw_schema_pane(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Schema;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.schema_loading {
        " Schema (loading...) "
    } else {
        " Schema "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    if app.visible_items.is_empty() && !app.schema_loading {
        let empty = Paragraph::new("No schema data")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app
        .visible_items
        .iter()
        .map(|item| {
            let style = match item.kind {
                ItemKind::Schema => Style::default().fg(Color::White).bold(),
                ItemKind::Table => Style::default().fg(Color::White),
                ItemKind::Column => Style::default().fg(Color::DarkGray),
            };
            ListItem::new(Line::from(Span::styled(&item.label, style)))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(Color::DarkGray));

    let mut state = app.schema_list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_editor_pane(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Editor;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut editor = app.editor.clone();
    editor.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" SQL Editor "),
    );

    if focused {
        editor.set_cursor_line_style(Style::default().bg(Color::DarkGray));
    } else {
        editor.set_cursor_line_style(Style::default());
    }

    f.render_widget(&editor, area);
}

fn draw_results_pane(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Results;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let status = if app.query_running {
        "Executing...".to_string()
    } else if app.result_status.is_empty() {
        String::new()
    } else {
        app.result_status.clone()
    };

    let title = if status.is_empty() {
        " Results ".to_string()
    } else {
        format!(" Results ({}) ", status)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    if app.result_columns.is_empty() {
        let empty = Paragraph::new(if app.query_running {
            "Running query..."
        } else {
            "Run a query with Ctrl+E"
        })
        .block(block)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(empty, area);
        return;
    }

    let header_cells: Vec<Cell> = app
        .result_columns
        .iter()
        .map(|h: &String| {
            Cell::from(h.as_str()).style(Style::default().fg(Color::Black).bg(Color::White).bold())
        })
        .collect();
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .result_rows
        .iter()
        .map(|row: &Vec<String>| {
            let cells: Vec<Cell> = row
                .iter()
                .map(|cell: &String| Cell::from(cell.as_str()))
                .collect();
            Row::new(cells)
        })
        .collect();

    let widths: Vec<Constraint> = app
        .result_columns
        .iter()
        .map(|_: &String| Constraint::Min(15))
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::default().bg(Color::DarkGray))
        .style(Style::default().fg(Color::White));

    let mut state = app.result_table_state.clone();
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_footer(app: &App) -> Paragraph<'static> {
    let focus_hint = match app.focus {
        FocusPane::Schema => "Schema",
        FocusPane::Editor => "Editor",
        FocusPane::Results => "Results",
    };

    Paragraph::new(Line::from(vec![
        Span::styled(" Tab", Style::default().fg(Color::Cyan)),
        Span::raw(" Switch Pane  "),
        Span::styled("Ctrl+E", Style::default().fg(Color::Cyan)),
        Span::raw(" Execute  "),
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(" Expand  "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" Back  "),
        Span::styled("Ctrl+C", Style::default().fg(Color::Cyan)),
        Span::raw(" Quit  \u{2502} Focus: "),
        Span::styled(focus_hint, Style::default().fg(Color::Yellow)),
    ]))
}
