//! Authoritative economy engine shared by the headless, HTTP, and native clients.
//! Rendering and wall-clock pacing must not change the order of simulation ticks.
pub mod bank;
pub mod city;
pub mod commands;
pub mod construction;
pub mod demographics;
pub mod education;
pub mod finance;
pub mod firms;
pub mod goods;
pub mod government;
pub mod homes;
pub mod ledger;
pub mod people;
pub mod physical;
pub mod politics;
pub mod rng;
pub mod save;
pub mod sim;
pub mod stats;

pub use sim::{Config, World};

pub mod traffic;
