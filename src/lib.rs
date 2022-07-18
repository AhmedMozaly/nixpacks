use crate::nixpacks::{
    app::App,
    builder::{
        docker::{DockerBuilder, DockerBuilderOptions},
        Builder,
    },
    environment::Environment,
    logger::Logger,
    nix::pkg::Pkg,
    plan::{
        generator::{GeneratePlanOptions, NixpacksBuildPlanGenerator},
        BuildPlan, PlanGenerator,
    },
};
use anyhow::Result;
use providers::{
    crystal::CrystalProvider, csharp::CSharpProvider, dart::DartProvider, deno::DenoProvider,
    fsharp::FSharpProvider, go::GolangProvider, haskell::HaskellStackProvider, java::JavaProvider,
    node::NodeProvider, php::PhpProvider, python::PythonProvider, ruby::RubyProvider,
    rust::RustProvider, staticfile::StaticfileProvider, swift::SwiftProvider, zig::ZigProvider,
    Provider,
};

mod chain;
#[macro_use]
pub mod nixpacks;
pub mod providers;

pub fn get_providers() -> Vec<&'static dyn Provider> {
    vec![
        &CrystalProvider {},
        &CSharpProvider {},
        &DartProvider {},
        &DenoProvider {},
        &FSharpProvider {},
        &GolangProvider {},
        &HaskellStackProvider {},
        &JavaProvider {},
        &PhpProvider {},
        &RubyProvider {},
        &NodeProvider {},
        &PythonProvider {},
        &RustProvider {},
        &SwiftProvider {},
        &StaticfileProvider {},
        &ZigProvider {},
    ]
}

pub fn generate_build_plan(
    path: &str,
    envs: Vec<&str>,
    plan_options: &GeneratePlanOptions,
) -> Result<BuildPlan> {
    let app = App::new(path)?;
    let environment = Environment::from_envs(envs)?;

    let mut generator = NixpacksBuildPlanGenerator::new(get_providers(), plan_options.to_owned());
    let plan = generator.generate_plan(&app, &environment)?;

    Ok(plan)
}

pub fn create_docker_image(
    path: &str,
    envs: Vec<&str>,
    plan_options: &GeneratePlanOptions,
    build_options: &DockerBuilderOptions,
) -> Result<()> {
    let app = App::new(path)?;
    let environment = create_environment(&app, envs)?;

    let mut generator = NixpacksBuildPlanGenerator::new(get_providers(), plan_options.to_owned());
    let plan = generator.generate_plan(&app, &environment)?;

    let logger = Logger::new();
    let builder = DockerBuilder::new(logger, build_options.to_owned());
    builder.create_image(app.source.to_str().unwrap(), &plan, &environment)?;

    Ok(())
}

pub fn create_environment(
    app: &App, 
    envs: Vec<&str>
) -> Result<Environment> {
    let environment = Environment::from_envs(envs)?;
    if app.includes_file(".env") {
        let procfile: HashMap<String, String> =
            app.read_yaml(".env").context("Reading Procfile")?;
        if let Some(release) = procfile.get("release") {
            Ok(Some(release.to_string()))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }

    return Ok(environment);
}