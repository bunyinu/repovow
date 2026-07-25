use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use std::io::stdout;

use crate::goal_edit::{load_form, save_goal, GoalForm};

const LABELS: [&str; 4] = [
    "Goal",
    "Current step",
    "Acceptance (one per line)",
    "Constraints (one per line)",
];

struct App {
    fields: [String; 4],
    selected: usize,
    status: String,
    dirty: bool,
}

impl App {
    fn from_form(form: GoalForm) -> Self {
        Self {
            fields: [
                form.title,
                form.step,
                form.acceptance.join("\n"),
                form.constraints.join("\n"),
            ],
            selected: 0,
            status: "↑↓ select field · Type to edit · Ctrl+S save · Esc quit".into(),
            dirty: false,
        }
    }

    fn to_form(&self) -> GoalForm {
        GoalForm {
            title: self.fields[0].clone(),
            step: self.fields[1].clone(),
            acceptance: GoalForm::parse_lines(&self.fields[2]),
            constraints: GoalForm::parse_lines(&self.fields[3]),
        }
    }

    fn save(&mut self) -> Result<()> {
        save_goal(&self.to_form(), None, "tui")?;
        self.dirty = false;
        self.status = format!("Saved goal: {}", self.fields[0].trim());
        Ok(())
    }
}

pub fn run_tui() -> Result<()> {
    let form = load_form(None).unwrap_or_default();
    let mut app = App::from_form(form);

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = loop {
        terminal.draw(|f| draw(f, &app))?;
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Esc => break Ok(()),
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Err(e) = app.save() {
                        app.status = format!("Error: {e}");
                    }
                }
                KeyCode::Up => {
                    app.selected = app.selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    app.selected = (app.selected + 1).min(LABELS.len() - 1);
                }
                KeyCode::Backspace => {
                    app.fields[app.selected].pop();
                    app.dirty = true;
                }
                KeyCode::Enter if app.selected < LABELS.len() - 1 => {
                    app.selected += 1;
                }
                KeyCode::Enter => {
                    if let Err(e) = app.save() {
                        app.status = format!("Error: {e}");
                    } else {
                        break Ok(());
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.fields[app.selected].push(c);
                    app.dirty = true;
                }
                _ => {}
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(f.area());

    let title = Paragraph::new("RepoVow — Goal editor")
        .style(Style::default().add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3); 4])
        .split(chunks[1]);

    for (i, area) in inner.iter().enumerate() {
        let focused = i == app.selected;
        let border_style = if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let label = Span::styled(
            format!("{}: ", LABELS[i]),
            Style::default().add_modifier(Modifier::BOLD),
        );
        let value = Span::raw(app.fields[i].clone());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        let para = Paragraph::new(Line::from(vec![label, value])).block(block);
        f.render_widget(para, *area);
    }

    let status_color = if app.status.starts_with("Error") {
        Color::Red
    } else if app.dirty {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let status = Paragraph::new(app.status.as_str())
        .style(Style::default().fg(status_color))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);
}
