use super::Provider;
use crate::nixpacks::{
    app::App,
    environment::Environment,
    nix::pkg::Pkg,
    phase::{BuildPhase, SetupPhase, StartPhase},
};
use anyhow::Result;
use regex::{Match, Regex};

const DEFAULT_JDK_PKG_NAME: &'static &str = &"jdk8";
pub struct ClojureProvider {}

impl Provider for ClojureProvider {
    fn name(&self) -> &str {
        "clojure"
    }

    fn detect(&self, app: &App, _env: &Environment) -> Result<bool> {
        Ok(app.includes_file("project.clj"))
    }

    fn setup(&self, _app: &App, _env: &Environment) -> Result<Option<SetupPhase>> {
        Ok(Some(SetupPhase::new(vec![
            Pkg::new("leiningen"),
            Pkg::new("jdk8"),
        ])))
    }

    fn build(&self, _app: &App, _env: &Environment) -> Result<Option<BuildPhase>> {
        Ok(Some(BuildPhase::new("lein uberjar".to_string())))
    }

    fn start(&self, _app: &App, _env: &Environment) -> Result<Option<StartPhase>> {
        Ok(Some(StartPhase::new(
            "java $JAVA_OPTS -jar target/uberjar/*standalone.jar".to_string(),
        )))
    }
}

impl ClojureProvider {
    fn get_nix_jdk_package(app: &App, env: &Environment) -> Result<Pkg> {
        // Fetch version from configs
        let mut custom_version = env.get_config_variable("JDK_VERSION");

        // If not from configs, get it from the .python-version file
        if custom_version.is_none() && app.includes_file(".jdk-version") {
            custom_version = Some(app.read_file(".jdk-version")?);
        }

        // If it's still none, return default
        if custom_version.is_none() {
            return Ok(Pkg::new(DEFAULT_JDK_PKG_NAME));
        }
        let custom_version = custom_version.unwrap();

        // Regex for reading Python versions (e.g. 3.8.0 or 3.8 or 3)
        let jdk_regex = Regex::new(r"^[0-9][0-9]?$")?;

        // Capture matches
        let matches = jdk_regex.captures(custom_version.as_str().trim());

        // If no matches, just use default
        if matches.is_none() {
            return Ok(Pkg::new(DEFAULT_JDK_PKG_NAME));
        }
        let matches = matches.unwrap();

        // Fetch python versions into tuples with defaults
        fn as_default(v: Option<Match>) -> &str {
            match v {
                Some(m) => m.as_str(),
                None => "_",
            }
        }
        let jdk_version = as_default(matches.get(0));
        // Match major and minor versions
        match jdk_version {
            "8" => Ok(Pkg::new(DEFAULT_JDK_PKG_NAME)),
            "11" => Ok(Pkg::new("jdk11")),
            _ => Ok(Pkg::new(DEFAULT_JDK_PKG_NAME)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::nixpacks::{app::App, environment::Environment, nix::pkg::Pkg};
    use std::collections::HashMap;

    #[test]
    fn test_no_version() -> Result<()> {
        assert_eq!(
            ClojureProvider::get_nix_jdk_package(
                &App::new("./examples/clojure")?,
                &Environment::default()
            )?,
            Pkg::new(DEFAULT_JDK_PKG_NAME)
        );

        Ok(())
    }

    #[test]
    fn test_custom_version() -> Result<()> {
        assert_eq!(
            ClojureProvider::get_nix_jdk_package(
                &App::new("./examples/clojure-jdk11")?,
                &Environment::default()
            )?,
            Pkg::new("jdk11")
        );

        Ok(())
    }

    #[test]
    fn test_version_from_environment_variable() -> Result<()> {
        assert_eq!(
            ClojureProvider::get_nix_jdk_package(
                &App::new("./examples/clojure")?,
                &Environment::new(HashMap::from([(
                    "NIXPACKS_JDK_VERSION".to_string(),
                    "11".to_string()
                )]))
            )?,
            Pkg::new("jdk11")
        );

        Ok(())
    }
}
