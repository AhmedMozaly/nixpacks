use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use super::Builder;
use crate::nixpacks::{
    app, cache::sanitize_cache_key, environment::Environment, files, logger::Logger, nix,
    plan::BuildPlan,
};
use anyhow::{bail, Context, Ok, Result};
use indoc::formatdoc;
use tempdir::TempDir;
use uuid::Uuid;

const DOT_NIXPACKS_DIR: &'static &str = &".nixpacks";

struct OutputDir {
    root_path: PathBuf,
    dockerfile_path: PathBuf,
    environment_nix_path: PathBuf,
}

impl OutputDir {
    pub fn new(root_path: PathBuf) -> Result<Self> {
        let dot_nixpacks_dir = PathBuf::from(&root_path)
            .join(PathBuf::from(DOT_NIXPACKS_DIR))
            .display()
            .to_string();

        if fs::metadata(&dot_nixpacks_dir).is_err() {
            fs::create_dir_all(&dot_nixpacks_dir)?;
        }

        let dockerfile_path = PathBuf::from(&dot_nixpacks_dir).join(PathBuf::from("Dockerfile"));
        let environment_nix_path =
            PathBuf::from(&dot_nixpacks_dir).join(PathBuf::from("environment.nix"));

        Ok(OutputDir {
            root_path,
            dockerfile_path,
            environment_nix_path,
        })
    }
}

#[derive(Clone, Default, Debug)]
pub struct DockerBuilderOptions {
    pub name: Option<String>,
    pub out_dir: Option<String>,
    pub print_dockerfile: bool,
    pub tags: Vec<String>,
    pub labels: Vec<String>,
    pub quiet: bool,
    pub cache_key: Option<String>,
    pub no_cache: bool,
    pub platform: Vec<String>,
}

pub struct DockerBuilder {
    logger: Logger,
    options: DockerBuilderOptions,
}

impl Builder for DockerBuilder {
    fn create_image(&self, app_src: &str, plan: &BuildPlan, env: &Environment) -> Result<()> {
        let id = Uuid::new_v4();

        let dir = match &self.options.out_dir {
            Some(dir) => dir.into(),
            None => {
                let tmp = TempDir::new("nixpacks").context("Creating a temp directory")?;
                tmp.into_path()
            }
        };
        let dest = dir.to_str().context("Invalid temp directory path")?;
        let output_dir = OutputDir::new(dir.clone()).context("Create .nixpacks directory")?;

        let name = self.options.name.clone().unwrap_or_else(|| id.to_string());

        // If printing the Dockerfile, don't write anything to disk
        if self.options.print_dockerfile {
            let dockerfile = self.create_dockerfile(plan, env);
            println!("{dockerfile}");

            return Ok(());
        }

        println!("{}", plan.get_build_string());

        // Write everything to destination
        self.write_app(app_src, dest).context("Writing app")?;
        self.write_assets(plan, dest).context("Writing assets")?;
        self.write_dockerfile(plan, &output_dir.dockerfile_path, env)
            .context("Writing Dockerfile")?;
        self.write_nix_expression(plan, &output_dir.environment_nix_path)
            .context("Writing NIx expression")?;

        // Only build if the --out flag was not specified
        if self.options.out_dir.is_none() {
            let mut docker_build_cmd =
                self.get_docker_build_cmd(plan, name.as_str(), &output_dir)?;

            let build_result = docker_build_cmd.spawn()?.wait().context("Building image")?;

            if !build_result.success() {
                bail!("Docker build failed")
            }

            self.logger.log_section("Successfully Built!");

            println!("\nRun:");
            println!("  docker run -it {}", name);
        } else {
            println!("\nSaved output to:");
            println!("  {}", dest);
        }

        Ok(())
    }
}

impl DockerBuilder {
    pub fn new(logger: Logger, options: DockerBuilderOptions) -> DockerBuilder {
        DockerBuilder { logger, options }
    }

    fn get_docker_build_cmd(
        &self,
        plan: &BuildPlan,
        name: &str,
        output_dir: &OutputDir,
    ) -> Result<Command> {
        let mut docker_build_cmd = Command::new("docker");

        if docker_build_cmd.output().is_err() {
            bail!("Please install Docker to build the app https://docs.docker.com/engine/install/")
        }

        // Enable BuildKit for all builds
        docker_build_cmd.env("DOCKER_BUILDKIT", "1");

        docker_build_cmd
            .arg("build")
            .arg(&output_dir.root_path)
            .arg("-t")
            .arg(name)
            .arg("-f")
            .arg(&output_dir.dockerfile_path);

        if self.options.quiet {
            docker_build_cmd.arg("--quiet");
        }

        if self.options.no_cache {
            docker_build_cmd.arg("--no-cache");
        }

        // Add build environment variables
        for (name, value) in plan.variables.clone().unwrap_or_default().iter() {
            docker_build_cmd
                .arg("--build-arg")
                .arg(format!("{}={}", name, value));
        }

        // Add user defined tags and labels to the image
        for t in self.options.tags.clone() {
            docker_build_cmd.arg("-t").arg(t);
        }
        for l in self.options.labels.clone() {
            docker_build_cmd.arg("--label").arg(l);
        }
        for l in self.options.platform.clone() {
            docker_build_cmd.arg("--platform").arg(l);
        }

        Ok(docker_build_cmd)
    }

    fn write_app(&self, app_src: &str, dest: &str) -> Result<()> {
        files::recursive_copy_dir(app_src, &dest)
    }

    fn write_dockerfile(
        &self,
        plan: &BuildPlan,
        dockerfile_path: &PathBuf,
        env: &Environment,
    ) -> Result<()> {
        let dockerfile = self.create_dockerfile(plan, env);

        File::create(dockerfile_path).context("Creating Dockerfile file")?;
        fs::write(dockerfile_path, dockerfile).context("Writing Dockerfile")?;

        Ok(())
    }

    fn write_nix_expression(&self, plan: &BuildPlan, environment_nix_path: &PathBuf) -> Result<()> {
        let nix_expression = nix::create_nix_expression(plan);

        let mut nix_file =
            File::create(environment_nix_path).context("Creating Nix environment file")?;
        nix_file
            .write_all(nix_expression.as_bytes())
            .context("Unable to write Nix expression")?;

        Ok(())
    }

    fn write_assets(&self, plan: &BuildPlan, dest: &str) -> Result<()> {
        if let Some(assets) = &plan.static_assets {
            if !assets.is_empty() {
                let static_assets_path = PathBuf::from(dest).join(PathBuf::from("assets"));
                fs::create_dir_all(&static_assets_path).context("Creating static assets folder")?;

                for (name, content) in assets {
                    let path = Path::new(&static_assets_path).join(name);
                    let parent = path.parent().unwrap();
                    fs::create_dir_all(parent)
                        .context(format!("Creating parent directory for {}", name))?;
                    let mut file =
                        File::create(path).context(format!("Creating asset file for {name}"))?;
                    file.write_all(content.as_bytes())
                        .context(format!("Writing asset {name}"))?;
                }
            }
        }

        Ok(())
    }

    fn create_dockerfile(&self, plan: &BuildPlan, env: &Environment) -> String {
        let environment_nix_path = PathBuf::from(DOT_NIXPACKS_DIR)
            .join(PathBuf::from("environment.nix"))
            .display()
            .to_string();

        let app_dir = "/app/";
        let assets_dir = app::ASSETS_DIR;

        let setup_phase = plan.setup.clone().unwrap_or_default();
        let install_phase = plan.install.clone().unwrap_or_default();
        let build_phase = plan.build.clone().unwrap_or_default();
        let start_phase = plan.start.clone().unwrap_or_default();
        let variables = plan.variables.clone().unwrap_or_default();
        let static_assets = plan.static_assets.clone().unwrap_or_default();

        let cache_key = if !self.options.no_cache && !env.is_config_variable_truthy("NO_CACHE") {
            self.options.cache_key.clone()
        } else {
            None
        };

        // -- Variables
        let args_string = if !variables.is_empty() {
            format!(
                "ARG {}\nENV {}",
                // Pull the variables in from docker `--build-arg`
                variables
                    .iter()
                    .map(|var| var.0.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                // Make the variables available at runtime
                variables
                    .iter()
                    .map(|var| format!("{}=${}", var.0, var.0))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        } else {
            "".to_string()
        };

        // -- Setup
        let mut setup_files: Vec<String> = vec![environment_nix_path];
        if let Some(mut setup_file_deps) = setup_phase.only_include_files {
            setup_files.append(&mut setup_file_deps);
        }
        let setup_copy_cmd = format!("COPY {} {}", setup_files.join(" "), app_dir);

        let mut apt_get_cmd = "".to_string();
        // using apt will break build reproducibility
        if !setup_phase.apt_pkgs.clone().unwrap_or_default().is_empty() {
            let apt_pkgs = setup_phase.apt_pkgs.unwrap_or_default().join(" ");
            apt_get_cmd = format!("RUN apt-get update && apt-get install -y {}", apt_pkgs);
        }
        let setup_cmd = setup_phase
            .cmds
            .unwrap_or_default()
            .iter()
            .map(|c| format!("RUN {}", c))
            .collect::<Vec<String>>()
            .join("\n");

        // -- Static Assets
        let assets_copy_cmd = if !static_assets.is_empty() {
            static_assets
                .into_keys()
                .map(|name| format!("COPY assets/{} {}{}", name, assets_dir, name))
                .collect::<Vec<String>>()
                .join("\n")
        } else {
            "".to_string()
        };

        // -- Install
        let install_cache_mount = get_cache_mount(&cache_key, &install_phase.cache_directories);

        let install_cmd = install_phase
            .cmds
            .unwrap_or_default()
            .iter()
            .map(|c| format!("RUN {} {}", install_cache_mount, c))
            .collect::<Vec<String>>()
            .join("\n");

        let (build_path, run_path) = if let Some(paths) = install_phase.paths {
            let joined_paths = paths.join(":");
            (
                format!("ENV PATH {}:$PATH", joined_paths),
                format!("RUN printf '\\nPATH={joined_paths}:$PATH' >> /root/.profile"),
            )
        } else {
            ("".to_string(), "".to_string())
        };

        // Files to copy for install phase
        // If none specified, copy over the entire app
        let install_files = install_phase
            .only_include_files
            .clone()
            .unwrap_or_else(|| vec![".".to_string()]);

        // -- Build
        let build_cache_mount = get_cache_mount(&cache_key, &build_phase.cache_directories);

        let build_cmd = build_phase
            .cmds
            .unwrap_or_default()
            .iter()
            .map(|c| format!("RUN {} {}", build_cache_mount, c))
            .collect::<Vec<String>>()
            .join("\n");

        let build_files = build_phase.only_include_files.unwrap_or_else(|| {
            // Only copy over the entire app if we haven't already in the install phase
            if install_phase.only_include_files.is_none() {
                Vec::new()
            } else {
                vec![".".to_string()]
            }
        });

        // -- Start
        let start_cmd = start_phase
            .cmd
            .map(|cmd| format!("CMD {}", cmd))
            .unwrap_or_else(|| "".to_string());

        // If we haven't yet copied over the entire app, do that before starting
        let start_files = start_phase.only_include_files.clone();

        let run_image_setup = match start_phase.run_image {
            Some(run_image) => {
                // RUN true to prevent a Docker bug https://github.com/moby/moby/issues/37965#issuecomment-426853382
                format! {"
                FROM {run_image}
                WORKDIR {app_dir}
                COPY --from=0 /etc/ssl/certs /etc/ssl/certs
                RUN true
                {copy_cmd}
            ",
                    run_image=run_image,
                    app_dir=app_dir,
                    copy_cmd=get_copy_from_command("0", &start_files.unwrap_or_default(), app_dir)
                }
            }
            None => get_copy_command(
                // If no files specified and no run image, copy everything in /app/ over
                &start_files.unwrap_or_else(|| vec![".".to_string()]),
                app_dir,
            ),
        };

        let dockerfile = formatdoc! {"
          FROM {base_image}

          WORKDIR {app_dir}

          # Setup
          {setup_copy_cmd}
          RUN nix-env -if environment.nix
          {apt_get_cmd}
          {setup_cmd}
          
          {assets_copy_cmd}

          # Load environment variables
          {args_string}

          # Install
          {install_copy_cmd}
          {install_cmd}

          {build_path}
          {run_path}

          # Build
          {build_copy_cmd}
          {build_cmd}

          # Start
          {run_image_setup}
          {start_cmd}
        ",
        base_image=setup_phase.base_image,
        install_copy_cmd=get_copy_command(&install_files, app_dir),
        build_copy_cmd=get_copy_command(&build_files, app_dir)};

        dockerfile
    }
}

fn get_cache_mount(cache_key: &Option<String>, cache_directories: &Option<Vec<String>>) -> String {
    match (cache_key, cache_directories) {
        (Some(cache_key), Some(cache_directories)) => cache_directories
            .iter()
            .map(|dir| {
                let sanitized_dir = dir.replace('~', "/root");
                let sanitized_key = sanitize_cache_key(format!("{cache_key}-{sanitized_dir}"));
                format!("--mount=type=cache,id={sanitized_key},target={sanitized_dir}")
            })
            .collect::<Vec<String>>()
            .join(" "),
        _ => "".to_string(),
    }
}

fn get_copy_command(files: &[String], app_dir: &str) -> String {
    if files.is_empty() {
        "".to_owned()
    } else {
        format!("COPY {} {}", files.join(" "), app_dir)
    }
}

fn get_copy_from_command(from: &str, files: &[String], app_dir: &str) -> String {
    if files.is_empty() {
        format!("COPY --from=0 {} {}", app_dir, app_dir)
    } else {
        format!(
            "COPY --from={} {} {}",
            from,
            files
                .iter()
                .map(|f| f.replace("./", app_dir))
                .collect::<Vec<_>>()
                .join(" "),
            app_dir
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cache_mount() {
        let cache_key = Some("cache_key".to_string());
        let cache_directories = Some(vec!["dir1".to_string(), "dir2".to_string()]);

        let expected = "--mount=type=cache,id=cache_key-dir1,target=dir1 --mount=type=cache,id=cache_key-dir2,target=dir2";
        let actual = get_cache_mount(&cache_key, &cache_directories);

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_get_cache_mount_invalid_cache_key() {
        let cache_key = Some("my cache key".to_string());
        let cache_directories = Some(vec!["dir1".to_string(), "dir2".to_string()]);

        let expected = "--mount=type=cache,id=my-cache-key-dir1,target=dir1 --mount=type=cache,id=my-cache-key-dir2,target=dir2";
        let actual = get_cache_mount(&cache_key, &cache_directories);

        assert_eq!(expected, actual);
    }
}
