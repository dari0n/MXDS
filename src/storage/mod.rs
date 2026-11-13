//! Локальное хранение: атомарная запись, журналы, файлы состояния.
//! Без БД (PROMPT.md: локальные файлы).

pub mod repository;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),
    #[error("ошибка сериализации: {0}")]
    Serialize(#[from] serde_json::Error),
}
