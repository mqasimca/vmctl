use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::cli::{CreateArgs, DiskMode, GetArgs, OutputFormat};
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
mod cache;
use cache::*;
mod iso;
use iso::*;
mod cloud;
use cloud::*;

pub(crate) use cache::{cache_lock, cache_prune_candidates, remove_cache_candidates};
pub(crate) use cloud::{create_cloud_seed, validate_hostname};
pub(super) use commands::{create, run};
pub(crate) use config_writer::{
    config_value, relative_value, validate_vm_name, write_new_config,
};

#[cfg(test)]
mod tests;
