use super::{dockerfile_generation::DockerfileGenerator, DockerBuilderOptions, ImageBuilder};
use crate::nixpacks::{
    builder::docker::dockerfile_generation::OutputDir, environment::Environment, files,
    logger::Logger, plan::BuildPlan,
};
use anyhow::{bail, Context, Ok, Result};

use std::{
    fmt::format,
    fs::{self, File},
    process::Command,
};
use tempdir::TempDir;
use uuid::Uuid;

pub struct DockerImageBuilder {
    logger: Logger,
    options: DockerBuilderOptions,
}

impl ImageBuilder for DockerImageBuilder {
    fn create_image(&self, app_src: &str, plan: &BuildPlan, env: &Environment) -> Result<()> {
        let id = Uuid::new_v4();

        let dir = match &self.options.out_dir {
            Some(dir) => dir.into(),
            None => {
                let tmp = TempDir::new("nixpacks").context("Creating a temp directory")?;
                tmp.into_path()
            }
        };
        let name = self.options.name.clone().unwrap_or_else(|| id.to_string());
        let output = OutputDir::new(dir)?;
        output.ensure_output_exists()?;

        let dockerfile = plan
            .generate_dockerfile(&self.options, env, &output)
            .context("Generating Dockerfile for plan")?;

        // If printing the Dockerfile, don't write anything to disk
        if self.options.print_dockerfile {
            println!("{}", dockerfile);
            return Ok(());
        }

        println!("{}", plan.get_build_string()?);

        self.write_app(app_src, &output).context("Writing app")?;
        self.write_dockerfile(dockerfile, &output)
            .context("Writing Dockerfile")?;
        plan.write_supporting_files(&self.options, env, &output)
            .context("Writing supporting files")?;

        // Only build if the --out flag was not specified
        if self.options.out_dir.is_none() {
            let mut docker_build_cmd = self.get_docker_build_cmd(plan, name.as_str(), &output)?;

            // Execute docker build
            let build_result = docker_build_cmd.spawn()?.wait().context("Building image")?;
            if !build_result.success() {
                bail!("Docker build failed")
            }

            self.logger.log_section("Successfully Built!");
            println!("\nRun:");
            println!("  docker run -it {}", name);
        } else {
            println!("\nSaved output to:");
            println!("  {}", output.root.to_str().unwrap());
        }

        Ok(())
    }
}

impl DockerImageBuilder {
    pub fn new(logger: Logger, options: DockerBuilderOptions) -> DockerImageBuilder {
        DockerImageBuilder { logger, options }
    }

    fn run_daemonless(&self, plan: &BuildPlan, output: &OutputDir, name: &str) -> Result<Command> {
        println!("Building with Buildkit in Daemonless mode");
        let mut docker_build_cmd = Command::new("docker");

        if docker_build_cmd.output().is_err() {
            bail!("Please install Docker to build the app https://docs.docker.com/engine/install/")
        }

        let target_dir = "/build-dir";
        let cache_dir = "/builder-files/buildkit-daemonless-cache2";

        docker_build_cmd
        .arg("run")
        .arg("-it")
        .arg("--privileged")
        .arg("-v")
        .arg(format!("{}:{}/", &output.root.display().to_string(), target_dir))
        .arg("-v")
        .arg(format!("{}:/cache-dir", cache_dir))
        .arg("--entrypoint")
        .arg("buildctl-daemonless.sh")
        .arg("moby/buildkit:master")
        .arg("build")
        .arg("--frontend")
        .arg("dockerfile.v0")
        .arg("--local")
        .arg(format!("context={}",target_dir))
        .arg("--local")
        .arg(format!("dockerfile={}/.nixpacks", target_dir))
        .arg("--import-cache")
        .arg("type=local,src=/cache-dir")
        .arg("--output")
        .arg(format!("type=image,name=us-west1-docker.pkg.dev/railway-infra-dev/railway-docker-internal-dev/{}", name))
        .arg("--export-cache")
        .arg("type=local,dest=/cache-dir,mode=max");

        Ok(docker_build_cmd)
    }

    fn run_kaniko(&self, plan: &BuildPlan, output: &OutputDir, name: &str) -> Result<Command> {
        println!("Building with  Kaniko");
        let mut docker_build_cmd = Command::new("docker");

        if docker_build_cmd.output().is_err() {
            bail!("Please install Docker to build the app https://docs.docker.com/engine/install/")
        }

        let context_dir = &output.root.display().to_string();
        println!("Context dir: {}", context_dir);

        let cache_dir = "/builder-files/kaniko-cache";
        let gcloud_idr = "/Users/ahmedmozaly";
        let container_root_dir = "/workspace";

        docker_build_cmd
            .arg("run")
            .arg("-v")
            .arg(format!(
                "{}/.config/gcloud:/root/.config/gcloud",
                gcloud_idr
            ))
            .arg("-v")
            .arg(format!("{}:{}", context_dir, container_root_dir))
            .arg("gcr.io/kaniko-project/executor:latest")
            .arg("--dockerfile")
            .arg(format!("{}/.nixpacks/Dockerfile", container_root_dir))
            .arg("--destination")
            .arg(format!("gcr.io/railway-infra-staging/{}", name.to_string()))
            .arg("--cache=true")
            .arg(format!("--cache-dir={}", cache_dir))
            .arg("--cache-copy-layers")
            .arg("--cache-run-layers")
            .arg("--context")
            .arg(container_root_dir);

        Ok(docker_build_cmd)
    }

    fn run_docker(&self, plan: &BuildPlan, output: &OutputDir, name: &str) -> Result<Command> {
        println!("Building with Buildkit");
        let mut docker_build_cmd = Command::new("docker");

        if docker_build_cmd.output().is_err() {
            bail!("Please install Docker to build the app https://docs.docker.com/engine/install/")
        }

        // Enable BuildKit for all buildsddd
        docker_build_cmd.env("DOCKER_BUILDKIT", "1");
        println!("output dir {}", &output.root.display().to_string());

        docker_build_cmd
            .arg("build")
            .arg(&output.root)
            .arg("-f")
            .arg(&output.get_absolute_path("Dockerfile"))
            .arg("-t")
            .arg(name);

        if self.options.quiet {
            docker_build_cmd.arg("--quiet");
        }

        if self.options.no_cache {
            docker_build_cmd.arg("--no-cache");
        }

        // Add build environment variables
        for (name, value) in &plan.variables.clone().unwrap_or_default() {
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

    fn get_docker_build_cmd(
        &self,
        plan: &BuildPlan,
        name: &str,
        output: &OutputDir,
    ) -> Result<Command> {
        self.run_kaniko(plan, output, name)
    }

    fn write_app(&self, app_src: &str, output: &OutputDir) -> Result<()> {
        files::recursive_copy_dir(app_src, &output.root)
    }

    fn write_dockerfile(&self, dockerfile: String, output: &OutputDir) -> Result<()> {
        let dockerfile_path = output.get_absolute_path("Dockerfile");
        File::create(dockerfile_path.clone()).context("Creating Dockerfile file")?;
        fs::write(dockerfile_path, dockerfile)?;

        Ok(())
    }
}
