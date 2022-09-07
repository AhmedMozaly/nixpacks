use std::collections::BTreeMap;

use super::Provider;
use crate::nixpacks::{
    app::App,
    environment::{Environment, EnvironmentVariables},
    nix::pkg::Pkg,
    plan::{
        phase::{Phase, StartPhase},
        BuildPlan,
    },
};
use anyhow::{bail, Result};
use regex::{Match, Regex};

const DEFAULT_JDK_PKG_NAME: &str = "jdk";

pub struct JavaProvider {}

impl Provider for JavaProvider {
    fn name(&self) -> &str {
        "Java"
    }

    fn detect(&self, app: &App, _env: &Environment) -> Result<bool> {
        Ok(self.is_maven_app(app)? || GradleHelper::is_gradle_app(app)?)
    }

    fn get_build_plan(&self, app: &App, env: &Environment) -> Result<Option<BuildPlan>> {
        let plan = if GradleHelper::is_gradle_app(app)? {
            GradleHelper::get_gradle_build_plan(app, env)?
        } else {
            self.get_maven_build_plan(app)
        };

        Ok(Some(plan))
    }
}

impl JavaProvider {
    fn get_maven_build_plan(&self, app: &App) -> BuildPlan {
        let setup = Phase::setup(Some(vec![Pkg::new("maven"), Pkg::new("jdk")]));

        let mvn_exe = self.get_maven_exe(app);
        let build = Phase::build(Some(format!("{mvn_exe} -DoutputFile=target/mvn-dependency-list.log -B -DskipTests clean dependency:list install", 
            mvn_exe=mvn_exe
        )));

        let start = StartPhase::new(self.get_start_cmd(app));

        BuildPlan::new(vec![setup, build], Some(start))
    }

    fn is_maven_app(&self, app: &App) -> Result<bool> {
        Ok(app.includes_file("pom.xml")
            || app.includes_directory("pom.atom")
            || app.includes_directory("pom.clj")
            || app.includes_directory("pom.groovy")
            || app.includes_file("pom.rb")
            || app.includes_file("pom.scala")
            || app.includes_file("pom.yaml")
            || app.includes_file("pom.yml"))
    }

    fn get_maven_exe(&self, app: &App) -> String {
        // App has a maven wrapper
        if app.includes_file("mvnw") && app.includes_file(".mvn/wrapper/maven-wrapper.properties") {
            "./mvnw".to_string()
        } else {
            "mvn".to_string()
        }
    }

    fn get_start_cmd(&self, app: &App) -> String {
        if app.includes_file("pom.xml") {
            format!(
                "java {} $JAVA_OPTS -jar target/*jar",
                self.get_port_config(app)
            )
        } else {
            "java $JAVA_OPTS -jar target/*jar".to_string()
        }
    }
    fn get_port_config(&self, app: &App) -> String {
        let pom_file = app.read_file("pom.xml").unwrap_or_default();
        if pom_file.contains("<groupId>org.wildfly.swarm") {
            "-Dswarm.http.port=$PORT".to_string()
        } else if pom_file.contains("<groupId>org.springframework.boot")
            && pom_file.contains("<artifactId>spring-boot")
        {
            "-Dserver.port=$PORT".to_string()
        } else {
            "".to_string()
        }
    }
}

struct GradleHelper {}
impl GradleHelper {
    pub fn is_gradle_app(app: &App) -> Result<bool> {
        let is_gralde = app.includes_file("gradlew");
        if !is_gralde {
            return Ok(false);
        }

        if !(app.includes_file("build.gradle") || app.includes_file("build.gradle.kts")) {
            bail!("Gradle project detected with invalid files, please make sure you have build.gradle or build.gradle.kts at the root of your project directory")
        }

        if !(app.includes_file("settings.gradle") || app.includes_file("settings.gradle")) {
            bail!("Gradle project detected with invalid files, please make sure you have build.gradle or build.gradle.kts at the root of your project directory")
        }

        if !(app.includes_file("gradle/wrapper/gradle-wrapper.properties")) {
            bail!("Gradle project detected with invalid files, please make sure you have gradle/wrapper/gradle-wrapper.properties in your project directory")
        }

        Ok(true)
    }

    pub fn get_gradle_build_plan(app: &App, env: &Environment) -> Result<BuildPlan> {
        let jdk_pkg = GradleHelper::get_jdk_pgk(app, env)?;
        let setup = Phase::setup(Some(vec![jdk_pkg]));

        let build = Phase::build(Some("./gradlew build -x check".to_string()));

        let start = StartPhase::new(
            "bash -c \"java -Dserver.port=$PORT $JAVA_OPTS -jar ./build/libs/*.jar\"",
        );

        let mut plan = BuildPlan::new(vec![setup, build], Some(start));

        // plan.add_variables(GradleHelper::get_gradle_env_vars(app)?);

        Ok(plan)
    }

    pub fn read_gradle_file(app: &App) -> Result<String> {
        if app.includes_file("build.gradle") {
            app.read_file("build.gradle")
        } else if app.includes_file("build.gradle.kts") {
            app.read_file("build.gradle")
        } else {
            Ok("".to_string())
        }
    }

    pub fn is_spring_boot(app: &App) -> Result<bool> {
        let file_content = GradleHelper::read_gradle_file(app)?;
        Ok(
            file_content.contains("org.springframework.boot:spring-boot")
                || file_content.contains("spring-boot-gradle-plugin")
                || file_content.contains("org.springframework.boot")
                || file_content.contains("org.grails:grails-"),
        )
    }

    pub fn get_gradle_env_vars(app: &App) -> Result<BTreeMap<String, String>> {
        let mut vars = EnvironmentVariables::from([(
            "GRADLE_OPTS".to_string(),
            "-Dorg.gradle.daemon=false -Dorg.gradle.internal.launcher.welcomeMessageEnabled=false"
                .to_string(),
        )]);

        if GradleHelper::is_spring_boot(app)? {
            let app_file_content = if app.includes_file("src/main/resources/config/application.yml")
            {
                app.read_file("src/main/resources/config/application.yml")?
            } else if app.includes_file("src/main/resources/config/application.properties") {
                app.read_file("src/main/resources/config/application.properties")?
            } else {
                "".to_string()
            };

            if app_file_content.len() > 0 {
                for captures in Regex::new(r#"\$\{(\w+)"#)?.captures_iter(&app_file_content) {
                    let key = captures.get(1).unwrap().as_str();
                    vars.insert(key.to_string(), "".to_string());
                }
            }
        }

        Ok(vars)
    }

    pub fn get_jdk_pgk(app: &App, env: &Environment) -> Result<Pkg> {
        let file_path = "gradle/wrapper/gradle-wrapper.properties";
        let file_path_override = ".gradle-version";
        let env_variable_name = "NIXPACKS_GRADLE_VERSION";
        let version_grouping_regex =
            Regex::new(r#"(distributionUrl[\S].*[gradle])(-)([0-9|\.]*)"#)?;
        let version_group_index = 3;
        let version_second_pass_regex =
            Regex::new(r#"^(?:[\sa-zA-Z-"']*)(\d*)(?:\.*)(\d*)(?:\.*\d*)(?:["']?)$"#)?;

        fn as_default(v: Option<Match>) -> &str {
            match v {
                Some(m) => m.as_str(),
                None => "_",
            }
        }

        let custom_version = env.get_config_variable(env_variable_name);

        // read from env variable > read from {file_path_override}  > read from {file_path}
        let custom_version = if custom_version.is_some() {
            custom_version
        } else if custom_version.is_none() && app.includes_file(file_path_override) {
            Some(app.read_file(file_path_override)?)
        } else {
            let file_content = app.read_file(file_path)?;
            version_grouping_regex
                .captures(&file_content)
                .map(|c| c.get(version_group_index).unwrap().as_str().to_owned())
        };

        // If it's still none, return default
        if custom_version.is_none() {
            return Ok(Pkg::new(DEFAULT_JDK_PKG_NAME));
        }
        let custom_version = custom_version.unwrap();

        let matches = version_second_pass_regex.captures(custom_version.as_str().trim());

        // If no matches, just use default
        if matches.is_none() {
            return Ok(Pkg::new(DEFAULT_JDK_PKG_NAME));
        }
        let matches = matches.unwrap();
        let parsed_version = as_default(matches.get(1));

        if parsed_version == "_".to_string() {
            return Ok(Pkg::new(DEFAULT_JDK_PKG_NAME));
        }

        let int_version = parsed_version.parse::<i32>().unwrap_or_default();
        let pkg = if int_version == 6 {
            Pkg::new("jdk11")
        } else if int_version < 6 {
            Pkg::new("jdk8")
        } else {
            Pkg::new("jdk")
        };

        Ok(pkg)
    }
}
