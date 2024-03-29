use std::borrow::Cow;
use std::env;
use std::path;
use std::path::Path;
use std::thread;

use crate::error;
use crate::error::MapLog;
use crate::error::Result;
use crate::structs;
use crate::util::LastPart;

pub(crate) fn process_current_dir(
    options: &structs::GetGitInfoOptions,
) -> Result<structs::GitOutputOptions> {
    let git_dir_buf =
        git_subfolder(options)?.ok_or_else(|| error::Error::from("Not found .git folder"))?;

    process_repo(&git_dir_buf, options)
}

fn git_subfolder(options: &structs::GetGitInfoOptions) -> Result<Option<path::PathBuf>> {
    let path = options
        .start_folder
        .as_ref()
        .map(Path::new)
        .map(Cow::from)
        .map(Ok)
        .unwrap_or_else(|| env::current_dir().map(Cow::from))?;

    if !path.exists() {
        return Err(format!("Path '{}' doesn't exist", path.display()).into());
    }

    for sub_path in path.ancestors() {
        let folder = sub_path.join(".git");
        if folder.exists() {
            return Ok(Some(sub_path.to_path_buf()));
        }
    }
    Ok(None)
}

fn process_repo(
    path: &Path,
    input_options: &structs::GetGitInfoOptions,
) -> Result<structs::GitOutputOptions> {
    let options = configuration_overrided(path, input_options)?;

    let mut head_info_result: Option<structs::GitHeadInfo> = None;
    let mut branch_ahead_behind_result: Option<structs::GitBranchAheadBehind> = None;
    let mut file_status_result: Option<structs::GitFileStatus> = None;

    thread::scope(|s| {
        s.spawn(|| {
            let repo_option = git2::Repository::open(path).ok_or_log();
            if repo_option.is_none() {
                return;
            };
            let repo = repo_option.unwrap();
            let head_info_internal = head_info(&repo, input_options.reference_name).ok_or_log();

            let ahead_behind = match options.include_ahead_behind {
                true => graph_ahead_behind(&repo, &head_info_internal).ok_or_log(),
                false => Some(structs::GitBranchAheadBehind {
                    ahead: 0,
                    behind: 0,
                }),
            };

            branch_ahead_behind_result = ahead_behind;
            head_info_result = head_info_internal.map(|h| h.into());
        });

        s.spawn(|| {
            let repo_option = git2::Repository::open(path).ok_or_log();
            if repo_option.is_none() {
                return;
            };
            let repo = repo_option.unwrap();
            file_status_result = file_status(&repo, &options).ok_or_log();
        });
    });

    Ok(structs::GitOutputOptions {
        head_info: head_info_result,
        file_status: file_status_result,
        branch_ahead_behind: branch_ahead_behind_result,
    })
}

#[derive(Debug)]
struct GitHeadInfoInternal {
    pub reference_name: Option<String>,
    pub oid: Option<git2::Oid>,
    pub detached: bool,
}

#[derive(Debug)]
struct GetGitInfoOptionsInternal {
    pub include_submodules: bool,
    pub include_untracked: bool,
    pub refresh_status: bool,
    pub include_ahead_behind: bool,
    pub include_workdir_stats: bool,
}

impl From<GitHeadInfoInternal> for structs::GitHeadInfo {
    fn from(val: GitHeadInfoInternal) -> Self {
        let reference_short = val
            .reference_name
            .map(|v| v.as_str().last_part().to_string());
        let oid_short = val.oid.map(|v| v.to_string()[0..8].to_string());

        structs::GitHeadInfo {
            reference_short,
            oid_short,
            detached: val.detached,
        }
    }
}

fn head_info(repo: &git2::Repository, input_reference_name: &str) -> Result<GitHeadInfoInternal> {
    let detached = repo.head_detached().unwrap_or_default();
    let reference = repo.find_reference(input_reference_name)?;

    let head_info = match reference.kind() {
        None => GitHeadInfoInternal {
            reference_name: None,
            oid: None,
            detached,
        },
        Some(git2::ReferenceType::Symbolic) => {
            let reference_name = reference.symbolic_target().map(String::from);

            let reference_resolved = reference.resolve().ok_or_log();
            let oid = reference_resolved.and_then(|r| r.target());

            GitHeadInfoInternal {
                reference_name,
                oid,
                detached,
            }
        }
        Some(git2::ReferenceType::Direct) => {
            let reference_name = reference.name().map(String::from);
            let oid = reference.target();

            GitHeadInfoInternal {
                reference_name,
                oid,
                detached,
            }
        }
    };
    Ok(head_info)
}

fn file_status(
    repo: &git2::Repository,
    options: &GetGitInfoOptionsInternal,
) -> Result<structs::GitFileStatus> {
    let status_options = &mut git2::StatusOptions::new();
    let status_show = match options.include_workdir_stats {
        true => git2::StatusShow::IndexAndWorkdir,
        false => git2::StatusShow::Index,
    };
    status_options.show(status_show);
    status_options.no_refresh(options.refresh_status);
    status_options.update_index(options.refresh_status);
    status_options.exclude_submodules(!options.include_submodules);
    status_options.include_ignored(false);
    status_options.include_unreadable(false);
    status_options.include_untracked(options.include_untracked);

    let statuses = repo.statuses(Some(status_options))?;

    let statuses_all = statuses
        .iter()
        .map(|s| s.status())
        .reduce(|a, b| a.union(b))
        .unwrap_or(git2::Status::empty());

    let mut conflict = false;
    let mut staged = false;
    let mut unstaged = false;
    let mut untracked = false;
    let mut typechange = false;

    for status in statuses_all {
        match status {
            git2::Status::CURRENT => conflict = true,
            git2::Status::INDEX_NEW => staged = true,
            git2::Status::INDEX_MODIFIED => staged = true,
            git2::Status::INDEX_DELETED => staged = true,
            git2::Status::INDEX_RENAMED => staged = true,
            git2::Status::INDEX_TYPECHANGE => staged = true,
            git2::Status::WT_NEW => untracked = true,
            git2::Status::WT_MODIFIED => unstaged = true,
            git2::Status::WT_DELETED => unstaged = true,
            git2::Status::WT_TYPECHANGE => typechange = true,
            git2::Status::WT_RENAMED => unstaged = true,
            git2::Status::IGNORED => (),
            git2::Status::CONFLICTED => conflict = true,
            _ => (),
        }
    }

    Ok(structs::GitFileStatus {
        conflict,
        untracked,
        typechange,
        unstaged,
        staged,
    })
}

fn graph_ahead_behind(
    repo: &git2::Repository,
    head: &Option<GitHeadInfoInternal>,
) -> Result<structs::GitBranchAheadBehind> {
    let reference: Option<&String> = head.as_ref().and_then(|h| h.reference_name.as_ref());
    let head_oid: Option<&git2::Oid> = head.as_ref().and_then(|h| h.oid.as_ref());

    if reference.is_none() || head_oid.is_none() {
        return Err("tracking branch doesn't exist".into());
    }

    let tracking_branch_buf = repo.branch_upstream_name(reference.unwrap())?;
    let tracking_branch = tracking_branch_buf.as_str();

    if tracking_branch.is_none() {
        return Err("tracking branch can't be converted to an UTF-8 string".into());
    }

    let tracking_reference = repo.find_reference(tracking_branch.unwrap())?;
    let tracking_oid = tracking_reference.target();

    if tracking_oid.is_none() {
        return Err("tracking branch {:?} has no oid".into());
    }

    let ahead_behind = repo.graph_ahead_behind(*head_oid.unwrap(), tracking_oid.unwrap())?;

    Ok(structs::GitBranchAheadBehind {
        ahead: ahead_behind.0,
        behind: ahead_behind.1,
    })
}

fn configuration_overrided(
    path: &Path,
    git_info_options: &structs::GetGitInfoOptions,
) -> Result<GetGitInfoOptionsInternal> {
    let repo = git2::Repository::open(path)?;
    let config = repo.config()?.snapshot()?;

    Ok(GetGitInfoOptionsInternal {
        include_submodules: config_bool_var(
            &config,
            "include-submodules",
            git_info_options.include_submodules,
        ),
        include_untracked: config_bool_var(
            &config,
            "include-untracked",
            git_info_options.include_untracked,
        ),
        refresh_status: config_bool_var(&config, "refresh-status", git_info_options.refresh_status),
        include_ahead_behind: config_bool_var(
            &config,
            "include-ahead-behind",
            git_info_options.include_ahead_behind,
        ),
        include_workdir_stats: config_bool_var(
            &config,
            "include-workdir-stats",
            git_info_options.include_workdir_stats,
        ),
    })
}

#[inline]
fn config_bool_var(config: &git2::Config, name: &'static str, default_value: bool) -> bool {
    config
        .get_bool(format!("{}.{}", env!("CARGO_BIN_NAME"), name).as_str())
        .unwrap_or(default_value)
}
