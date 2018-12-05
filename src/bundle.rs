use std::io::{self, Write};
use std::fs::{self, File};
use std::path::Path;

use cargo::{Config, CargoResult};
use cargo::core::{Package, Shell};

use license::License;
use licensed::Licensed;
use options::Bundle;
use discovery::{
    Confidence, LicenseText, find_generic_license_text, find_license_text
};

struct Context<'a> {
    roots_name: String,
    packages: &'a [Package],
    shell: &'a mut Shell,

    missing_license: bool,
    low_quality_license: bool,
}

pub fn run(roots: &[Package], mut packages: Vec<Package>, config: &Config, variant: Bundle) -> CargoResult<()> {
    packages.sort_by_key(|package| package.name().to_owned());

    let roots_name = {
        if roots.len() == 1 {
            format!("{} package", roots[0].name())
        } else {
            let mut roots_name = String::new();
            roots_name += roots[0].name().as_str();
            for root in roots.iter().take(roots.len() - 1).skip(1) {
                roots_name += ", ";
                roots_name += root.name().as_str();
            }
            roots_name += " and ";
            roots_name += roots.last().unwrap().name().as_str();
            roots_name += " packages";
            roots_name
        }
    };
    let mut context = Context {
        roots_name,
        packages: &packages,
        shell: &mut config.shell(),
        missing_license: false,
        low_quality_license: false,
    };

    match variant {
        Bundle::Inline { file } => {
            if let Some(file) = file {
                inline(&mut context, &mut File::create(file)?)?;
            } else {
                inline(&mut context, &mut io::stdout())?;
            }
        }
        Bundle::NameOnly { file } => {
            if let Some(file) = file {
                name_only(&mut context, &mut File::create(file)?)?;
            } else {
                name_only(&mut context, &mut io::stdout())?;
            }
        }
        Bundle::Source { file } => {
            if let Some(file) = file {
                source(&mut context, &mut File::create(file)?)?;
            } else {
                source(&mut context, &mut io::stdout())?;
            }
        }
        Bundle::Split { file, dir } => {
            if let Some(file) = file {
                split(&mut context, &mut File::create(file)?, dir)?;
            } else {
                split(&mut context, &mut io::stdout(), dir)?;
            }
        }
    }

    if context.missing_license {
        context.shell.error("
  Our liches failed to recognise a license in one or more packages.

  We would be very grateful if you could check the corresponding package
  directories (see the package specific message above) to see if there is an
  easily recognisable license file available.

  If there is please submit details to
      https://github.com/Nemo157/cargo-lichking/issues
  so we can make sure this license is recognised in the future.

  If there isn't you could submit an issue to the package's project asking
  them to include the text of their license in the built packages.")?;
    }

    if context.low_quality_license {
        context.shell.error("\
  Our liches are very unsure about one or more licenses that were put into the \
  bundle. Please check the specific error messages above.")?;
    }

    if context.missing_license || context.low_quality_license {
        bail!("Generating bundle finished with error(s)")
    } else {
        Ok(())
    }
}

fn inline(context: &mut Context, out: &mut io::Write) -> CargoResult<()> {
    writeln!(out, "The {} uses some third party libraries under their own license terms:", context.roots_name)?;
    writeln!(out)?;
    for package in context.packages {
        writeln!(out, " * {} {} under the terms of {}:", package.name(), package.version(), package.license())?;
        writeln!(out)?;
        inline_package(context, package, out)?;
        writeln!(out)?;
    }
    Ok(())
}

fn name_only(context: &mut Context, out: &mut io::Write) -> CargoResult<()> {
    writeln!(out, "The {} uses some third party libraries under their own license terms:", context.roots_name)?;
    writeln!(out)?;
    for package in context.packages {
        writeln!(out, " * {} {} under the terms of {}", package.name(), package.version(), package.license())?;
    }
    Ok(())
}

fn source(context: &mut Context, out: &mut io::Write) -> CargoResult<()> {
    out.write_all(b"
//! Licenses of dependencies
//!
//! This file was generated by [`cargo-lichking`](https://github.com/Nemo157/cargo-lichking)

pub struct License {
    pub name: &'static str,
    pub text: Option<&'static str>,
}

pub struct Licenses {
    pub name: &'static str,
    pub licenses: &'static [License],
}

pub struct LicensedCrate {
    pub name: &'static str,
    pub version: &'static str,
    pub licenses: Licenses,
}

pub const CRATES: &'static [LicensedCrate] = &[
")?;
    for package in context.packages {
        source_package(context, package, out)?;
    }
    out.write_all(b"];")?;
    Ok(())
}

fn split<P: AsRef<Path>>(context: &mut Context, out: &mut io::Write, dir: P) -> CargoResult<()> {
    fs::create_dir_all(dir.as_ref())?;
    writeln!(out, "The {} uses some third party libraries under their own license terms:", context.roots_name)?;
    writeln!(out)?;
    for package in context.packages {
        writeln!(out, " * {} {} under the terms of {}", package.name(), package.version(), package.license())?;
        split_package(context, package, dir.as_ref())?;
    }
    Ok(())
}

fn inline_package(context: &mut Context, package: &Package, out: &mut io::Write) -> CargoResult<()> {
    let license = package.license();
    if let Some(text) = find_generic_license_text(package, &license)? {
        match text.confidence {
            Confidence::Confident => (),
            Confidence::SemiConfident => {
                context.shell.warn(format_args!("{} has only a low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.warn(format_args!("    {}", text.path.display()))?;
            }
            Confidence::Unsure => {
                context.shell.error(format_args!("{} has only a very low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
        }
        for line in text.text.lines() {
            writeln!(out, "    {}", line)?;
        }
    } else {
        match license {
            License::Unspecified => {
                context.shell.error(format_args!("{} does not specify a license", package.name()))?;
            }
            License::Multiple(licenses) => {
                let mut first = true;
                for license in licenses {
                    if first {
                        first = false;
                    } else {
                        writeln!(out)?;
                        writeln!(out, "    ===============")?;
                        writeln!(out)?;
                    }
                    inline_license(context, package, &license, out)?;
                }
            }
            license => {
                inline_license(context, package, &license, out)?;
            }
        }
    }
    writeln!(out)?;
    Ok(())
}

fn source_package(context: &mut Context, package: &Package, out: &mut io::Write) -> CargoResult<()> {
    let license = package.license();
    if let Some(text) = find_generic_license_text(package, &license)? {
        match text.confidence {
            Confidence::Confident => (),
            Confidence::SemiConfident => {
                context.shell.warn(format_args!("{} has only a low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.warn(format_args!("    {}", text.path.display()))?;
            }
            Confidence::Unsure => {
                context.shell.error(format_args!("{} has only a very low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
        }
        writeln!(out, "
    LicensedCrate {{
        name: {:?},
        version: {:?},
        licenses: Licenses {{
            name: {:?},
            licenses: &[
                License {{
                    name: {:?},
                    text: Some({:?}),
                }},
            ],
        }},
    }},", package.name(), package.version().to_string(), license.to_string(), license.to_string(), text.text)?;
    } else {
        let license_name = license.to_string();
        match license {
            License::Unspecified => {
                context.shell.error(format_args!("{} does not specify a license", package.name()))?;
            }
            License::Multiple(licenses) => {
                writeln!(out, "
    LicensedCrate {{
        name: {:?},
        version: {:?},
        licenses: Licenses {{
            name: {:?},
            licenses: &[", package.name(), package.version().to_string(), license_name)?;
                for license in licenses {
                    let texts = find_license_text(package, &license)?;
                    let text = (choose(context, package, &license, texts)?).map(|t| format!("Some({:?})", t.text)).unwrap_or_else(|| "None".to_owned());
                    writeln!(out, "
                License {{
                    name: {:?},
                    text: {},
                }},", license.to_string(), text)?;
                }
                writeln!(out, "
            ],
        }},
    }},")?;
            }
            license => {
                let texts = find_license_text(package, &license)?;
                let text = (choose(context, package, &license, texts)?).map(|t| format!("Some({:?})", t.text)).unwrap_or_else(|| "None".to_owned());
                writeln!(out, "
    LicensedCrate {{
        name: {:?},
        version: {:?},
        licenses: Licenses {{
            name: {:?},
            licenses: &[
                License {{
                    name: {:?},
                    text: {},
                }},
            ],
        }},
    }},", package.name(), package.version().to_string(), license.to_string(), license.to_string(), text)?;
            }
        }
    }
    writeln!(out)?;
    Ok(())
}

fn split_package(context: &mut Context, package: &Package, dir: &Path) -> CargoResult<()> {
    let license = package.license();
    let mut file = File::create(dir.join(package.name().as_str()))?;
    if let Some(text) = find_generic_license_text(package, &license)? {
        match text.confidence {
            Confidence::Confident => (),
            Confidence::SemiConfident => {
                context.shell.warn(format_args!("{} has only a low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.warn(format_args!("    {}", text.path.display()))?;
            }
            Confidence::Unsure => {
                context.shell.error(format_args!("{} has only a very low-confidence candidate for license {}:", package.name(), license))?;
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
        }
        file.write_all(text.text.as_bytes())?;
    } else {
        match license {
            License::Unspecified => {
                context.shell.error(format_args!("{} does not specify a license", package.name()))?;
            }
            License::Multiple(licenses) => {
                let mut first = true;
                for license in licenses {
                    if first {
                        first = false;
                    } else {
                        writeln!(file)?;
                        writeln!(file, "===============")?;
                        writeln!(file)?;
                    }
                    let texts = find_license_text(package, &license)?;
                    if let Some(text) = choose(context, package, &license, texts)? {
                        file.write_all(text.text.as_bytes())?;
                    }
                }
            }
            license => {
                let texts = find_license_text(package, &license)?;
                if let Some(text) = choose(context, package, &license, texts)? {
                    file.write_all(text.text.as_bytes())?;
                }
            }
        }
    }
    Ok(())
}

fn inline_license(context: &mut Context, package: &Package, license: &License, out: &mut io::Write) -> CargoResult<()> {
    let texts = find_license_text(package, license)?;
    if let Some(text) = choose(context, package, license, texts)? {
        for line in text.text.lines() {
            writeln!(out, "    {}", line)?;
        }
    }
    Ok(())
}

fn choose(context: &mut Context, package: &Package, license: &License, texts: Vec<LicenseText>) -> CargoResult<Option<LicenseText>> {
    let (mut confident, texts): (Vec<LicenseText>, Vec<LicenseText>) = texts.into_iter().partition(|text| text.confidence == Confidence::Confident);
    let (mut semi_confident, mut unconfident): (Vec<LicenseText>, Vec<LicenseText>) = texts.into_iter().partition(|text| text.confidence == Confidence::SemiConfident);

    Ok(Some({
        if confident.len() == 1 {
            confident.swap_remove(0)
        } else if confident.len() > 1 {
            context.shell.error(format_args!("{} has multiple candidates for license {}:", package.name(), license))?;
            for text in &confident {
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
            confident.swap_remove(0)
        } else if semi_confident.len() == 1 {
            context.shell.warn(format_args!("{} has only a low-confidence candidate for license {}:", package.name(), license))?;
            context.shell.warn(format_args!("    {}", semi_confident[0].path.display()))?;
            semi_confident.swap_remove(0)
        } else if semi_confident.len() > 1 {
            context.low_quality_license = true;
            context.shell.error(format_args!("{} has multiple low-confidence candidates for license {}:", package.name(), license))?;
            for text in &semi_confident {
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
            semi_confident.swap_remove(0)
        } else if unconfident.len() == 1 {
            context.low_quality_license = true;
            context.shell.warn(format_args!("{} has only a very low-confidence candidate for license {}:", package.name(), license))?;
            context.shell.warn(format_args!("    {}", unconfident[0].path.display()))?;
            unconfident.swap_remove(0)
        } else if unconfident.len() > 1 {
            context.low_quality_license = true;
            context.shell.error(format_args!("{} has multiple very low-confidence candidates for license {}:", package.name(), license))?;
            for text in &unconfident {
                context.shell.error(format_args!("    {}", text.path.display()))?;
            }
            unconfident.swap_remove(0)
        } else {
            context.shell.error(format_args!("{} has no candidate texts for license {} in {}", package.name(), license, package.root().display()))?;
            context.missing_license = true;
            return Ok(None);
        }
    }))
}
