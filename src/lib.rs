#![warn(clippy::pedantic)]
#![allow(
    // Allowed as they are too pedantic.
    clippy::cast_possible_truncation,
    clippy::unreadable_literal,
    clippy::cast_possible_wrap,
    clippy::wildcard_imports,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::cast_lossless,
    clippy::unused_self,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    // TODO: Remove when everything is documented.
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
)]

use crate::nixpacks::{
    app::App,
    builder::{docker::docker_image_builder::DockerImageBuilder, ImageBuilder},
    environment::Environment,
    logger::Logger,
    nix::pkg::Pkg,
    plan::{generator::NixpacksBuildPlanGenerator, BuildPlan, PlanGenerator},
};

use anyhow::{bail, Result};
use nixpacks::{
    builder::docker::{utils, DockerBuilderOptions},
    plan::generator::GeneratePlanOptions,
};
use providers::{
    clojure::ClojureProvider, crystal::CrystalProvider, csharp::CSharpProvider, dart::DartProvider,
    deno::DenoProvider, elixir::ElixirProvider, fsharp::FSharpProvider, go::GolangProvider,
    haskell::HaskellStackProvider, java::JavaProvider, node::NodeProvider, php::PhpProvider,
    python::PythonProvider, ruby::RubyProvider, rust::RustProvider, staticfile::StaticfileProvider,
    swift::SwiftProvider, zig::ZigProvider, Provider,
};

mod chain;
#[macro_use]
pub mod nixpacks;
pub mod providers;

pub fn get_providers() -> &'static [&'static dyn Provider] {
    &[
        &CrystalProvider {},
        &CSharpProvider {},
        &DartProvider {},
        &ElixirProvider {},
        &DenoProvider {},
        &FSharpProvider {},
        &ClojureProvider {},
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
    options: &GeneratePlanOptions,
) -> Result<BuildPlan> {
    let app = App::new(path)?;
    let environment = Environment::from_envs(envs)?;

    let mut generator = NixpacksBuildPlanGenerator::new(get_providers(), options.clone());
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
    let environment = Environment::from_envs(envs)?;

    let mut generator = NixpacksBuildPlanGenerator::new(get_providers(), plan_options.clone());
    let plan = generator.generate_plan(&app, &environment)?;

    if let Some(ref phase) = plan.start_phase {
        if phase.cmd.is_none() && !build_options.no_error_without_start {
            bail!("No start command could be found")
        }
    }

    let logger = Logger::new();
    let builder = DockerImageBuilder::new(logger, build_options.clone());

    builder.create_image(app.source.to_str().unwrap(), &plan, &environment)?;

    Ok(())
}

// async fn save_file(mut payload: Multipart, save_to: String) -> Result<HttpResponse, Error> {
//     // iterate over multipart stream
//     while let Some(mut field) = payload.try_next().await? {
//         // A multipart/form-data stream has to contain `content_disposition`
//         let content_disposition = field.content_disposition();

//         let filename = content_disposition
//             .get_filename()
//             .map_or_else(|| Uuid::new_v4().to_string(), sanitize_filename::sanitize);
//         let filepath = format!("{save_to}/{filename}");

//         // File::create is blocking operation, use threadpool
//         let mut f = web::block(|| std::fs::File::create(filepath)).await??;

//         // Field in turn is stream of *Bytes* object
//         while let Some(chunk) = field.try_next().await? {
//             // filesystem operations are blocking, we have to use threadpool
//             f = web::block(move || f.write_all(&chunk).map(|_| f)).await??;
//         }

//     }

//     Ok(HttpResponse::Ok().into())
// }

// fn extract_files(filepath: String){
//     let file = File::open(filepath)?;
//     let mut archive = Archive::new(GzDecoder::new(file));
//     archive
//     .entries()?
//     .filter_map(|e| e.ok())
//     .map(|mut entry| -> Result<PathBuf> {
//         let path = entry.path()?.strip_prefix(prefix)?.to_owned();
//         entry.unpack(&path)?;
//         Ok(path)
//     })
//     .filter_map(|e| e.ok())
//     .for_each(|x| println!("> {}", x.display()));
// }
