use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::cli::{GetArgs, OutputFormat};
use crate::error::{Error, Result};
use crate::paths::Dirs;

mod catalog;
use catalog::*;
mod commands;
use commands::*;
mod download;
use download::*;
mod output;
use output::*;
mod resolver;
use resolver::*;
mod remote;
use remote::*;
mod assets_a_k;
use assets_a_k::*;
mod macos;
use macos::*;
mod windows;
use windows::*;
mod assets_l_s;
use assets_l_s::*;
mod assets_t_z;
use assets_t_z::*;
mod io;
use io::*;
mod config_writer;
use config_writer::*;

pub(super) use commands::run;

#[cfg(test)]
mod tests;
