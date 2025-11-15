use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    io,
    time::{Duration, Instant},
};
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction as LayoutDirection, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Position {
    x: u16,
    y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SnakeDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug)]
struct Snake {
    body: Vec<Position>,
    direction: SnakeDirection,
    growing: bool,
}

impl Snake {
    fn new(start_x: u16, start_y: u16) -> Self {
        Self {
            body: vec![
                Position { x: start_x, y: start_y },
                Position { x: start_x - 1, y: start_y },
                Position { x: start_x - 2, y: start_y },
            ],
            direction: SnakeDirection::Right,
            growing: false,
        }
    }

    fn head(&self) -> Position {
        self.body[0]
    }

    fn change_direction(&mut self, new_direction: SnakeDirection) {
        // 防止反向移动
        let valid = match (self.direction, new_direction) {
            (SnakeDirection::Up, SnakeDirection::Down) | (SnakeDirection::Down, SnakeDirection::Up) => false,
            (SnakeDirection::Left, SnakeDirection::Right) | (SnakeDirection::Right, SnakeDirection::Left) => false,
            _ => true,
        };

        if valid {
            self.direction = new_direction;
        }
    }

    fn move_forward(&mut self, width: u16, height: u16) -> bool {
        let head = self.head();
        let new_head = match self.direction {
            SnakeDirection::Up => Position {
                x: head.x,
                y: if head.y == 0 { height - 1 } else { head.y - 1 },
            },
            SnakeDirection::Down => Position {
                x: head.x,
                y: (head.y + 1) % height,
            },
            SnakeDirection::Left => Position {
                x: if head.x == 0 { width - 1 } else { head.x - 1 },
                y: head.y,
            },
            SnakeDirection::Right => Position {
                x: (head.x + 1) % width,
                y: head.y,
            },
        };

        // 检查是否撞到自己
        if self.body.contains(&new_head) {
            return false;
        }

        self.body.insert(0, new_head);

        if !self.growing {
            self.body.pop();
        } else {
            self.growing = false;
        }

        true
    }

    fn grow(&mut self) {
        self.growing = true;
    }

    fn collides_with(&self, pos: Position) -> bool {
        self.body.contains(&pos)
    }
}

struct Game {
    snake: Snake,
    foods: Vec<Position>,
    score: u32,
    game_over: bool,
    paused: bool,
    width: u16,
    height: u16,
}

impl Game {
    fn new(width: u16, height: u16) -> Self {
        let snake = Snake::new(width / 2, height / 2);
        let mut game = Self {
            snake,
            foods: Vec::new(),
            score: 0,
            game_over: false,
            paused: false,
            width,
            height,
        };
        game.spawn_food();
        game
    }

    fn spawn_food(&mut self) {
        use rand::Rng;
        let mut rng = rand::rng();
        
        // Limit to 6 foods max
        if self.foods.len() >= 6 {
            return;
        }

        loop {
            let pos = Position {
                x: rng.random_range(0..self.width),
                y: rng.random_range(0..self.height),
            };

            // Check if position conflicts with snake or existing foods
            if !self.snake.collides_with(pos) && !self.foods.contains(&pos) {
                self.foods.push(pos);
                break;
            }
        }
    }

    fn update(&mut self) {
        if self.game_over || self.paused {
            return;
        }

        if !self.snake.move_forward(self.width, self.height) {
            self.game_over = true;
            return;
        }

        // Check if snake ate any food
        let head = self.snake.head();
        if let Some(index) = self.foods.iter().position(|&food| food == head) {
            self.snake.grow();
            self.score += 10;
            self.foods.remove(index);
            self.spawn_food();
            
            // Try to spawn another food occasionally for more dynamic gameplay
            if self.foods.len() < 3 && rand::random::<u8>() % 3 == 0 {
                self.spawn_food();
            }
        }
    }

    fn toggle_pause(&mut self) {
        if !self.game_over {
            self.paused = !self.paused;
        }
    }

    fn reset(&mut self) {
        self.snake = Snake::new(self.width / 2, self.height / 2);
        self.foods.clear();
        self.score = 0;
        self.game_over = false;
        self.paused = false;
        self.spawn_food();
    }
}

pub async fn run_game() -> Result<()> {
    // 设置终端
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 创建游戏
    let game_area = terminal.size()?;
    let game_width = game_area.width.saturating_sub(4).max(20);
    let game_height = game_area.height.saturating_sub(6).max(10);
    let mut game = Game::new(game_width, game_height);

    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(150);

    // 游戏循环
    loop {
        terminal.draw(|f| draw_ui(f, &game))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('r') if game.game_over => game.reset(),
                    KeyCode::Char(' ') => game.toggle_pause(),
                    KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('k') => {
                        game.snake.change_direction(SnakeDirection::Up)
                    }
                    KeyCode::Down | KeyCode::Char('s') | KeyCode::Char('j') => {
                        game.snake.change_direction(SnakeDirection::Down)
                    }
                    KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('h') => {
                        game.snake.change_direction(SnakeDirection::Left)
                    }
                    KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('l') => {
                        game.snake.change_direction(SnakeDirection::Right)
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            game.update();
            last_tick = Instant::now();
        }
    }

    // 恢复终端
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("\n🎮 Game Over! Final Score: {}", game.score);
    if game.score >= 100 {
        println!("I am Ayin, I love LCX");
    }
    println!("Thanks for playing!\n");

    Ok(())
}

fn draw_ui<B: tui::backend::Backend>(f: &mut Frame<B>, game: &Game) {
    let chunks = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.size());

    // 标题
    let title = Paragraph::new(vec![Spans::from(vec![
        Span::styled("🐍 ", Style::default().fg(Color::Green)),
        Span::styled("SURF SNAKE GAME", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" 🎮", Style::default().fg(Color::Yellow)),
    ])])
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).style(Style::default().fg(Color::White)));
    f.render_widget(title, chunks[0]);

    // 游戏区域
    let game_area = chunks[1];
    draw_game_area(f, game, game_area);

    // 底部信息
    let status_text = if game.game_over {
        let score_str = format!("Score: {} ", game.score);
        vec![
            Spans::from(vec![
                Span::styled("GAME OVER! ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled(score_str, Style::default().fg(Color::Yellow)),
                Span::raw("| Press "),
                Span::styled("R", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(" to restart | "),
                Span::styled("Q", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(" to quit"),
            ])
        ]
    } else if game.paused {
        vec![
            Spans::from(vec![
                Span::styled("PAUSED ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("| Press "),
                Span::styled("SPACE", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(" to continue"),
            ])
        ]
    } else {
        let score_str = format!("{} ", game.score);
        vec![
            Spans::from(vec![
                Span::styled("Score: ", Style::default().fg(Color::White)),
                Span::styled(score_str, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("| "),
                Span::styled("Arrow/WASD/HJKL", Style::default().fg(Color::Green)),
                Span::raw(": Move | "),
                Span::styled("SPACE", Style::default().fg(Color::Cyan)),
                Span::raw(": Pause | "),
                Span::styled("Q", Style::default().fg(Color::Red)),
                Span::raw(": Quit"),
            ])
        ]
    };

    let status = Paragraph::new(status_text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[2]);
}

fn draw_game_area<B: tui::backend::Backend>(f: &mut Frame<B>, game: &Game, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Draw foods
    for food in &game.foods {
        if food.x < inner.width && food.y < inner.height {
            let food_cell = Rect {
                x: inner.x + food.x,
                y: inner.y + food.y,
                width: 1,
                height: 1,
            };
            let food = Paragraph::new("●")
                .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
            f.render_widget(food, food_cell);
        }
    }

    // 绘制蛇
    for (i, pos) in game.snake.body.iter().enumerate() {
        if pos.x < inner.width && pos.y < inner.height {
            let cell = Rect {
                x: inner.x + pos.x,
                y: inner.y + pos.y,
                width: 1,
                height: 1,
            };

            let (symbol, color) = if i == 0 {
                ("◉", Color::Green)
            } else {
                ("■", Color::LightGreen)
            };

            let segment = Paragraph::new(symbol)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD));
            f.render_widget(segment, cell);
        }
    }

    // 游戏结束或暂停时的遮罩
    if game.game_over || game.paused {
        let overlay_area = Rect {
            x: inner.x + inner.width / 4,
            y: inner.y + inner.height / 2 - 2,
            width: inner.width / 2,
            height: 4,
        };

        let overlay_text = if game.game_over {
            let final_score = format!("Final Score: {}", game.score);
            vec![
                Spans::from(Span::styled(
                    "GAME OVER! ",
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::BOLD)
                        .add_modifier(Modifier::SLOW_BLINK),
                )),
                Spans::from(Span::styled(
                    final_score,
                    Style::default().fg(Color::Yellow),
                )),
            ]
        } else {
            vec![
                Spans::from(Span::styled(
                    "⏸ PAUSED ⏸",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
            ]
        };

        let overlay = Paragraph::new(overlay_text)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::White).bg(Color::Black)),
            );
        f.render_widget(overlay, overlay_area);
    }
}