#![forbid(unsafe_code)]
//! learn.xabcnews.com: courses on AI and hardware that XABC writes and keeps current.

mod ai;
pub mod commands;
pub mod config;
mod courses;
pub mod error;
mod markdown;
mod media;
mod pages;
pub mod state;
pub mod web;
pub mod writer;

#[cfg(test)]
mod tests;
