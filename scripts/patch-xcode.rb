#!/usr/bin/env ruby
# frozen_string_literal: true

# Patch the Xcode project to link Rust static libs and include UniFFI bindings.
require "xcodeproj"

project = Xcodeproj::Project.open("ManisPocket.xcodeproj")
target = project.targets.find { |t| t.name == "ManisPocket" }
abort "Target 'ManisPocket' not found" unless target

# Create module map for C FFI header
File.write("ManisPocket/Generated/module.modulemap", <<~MODULEMAP)
  module ManisPocketCoreFFI {
      header "ManisPocketCoreFFI.h"
      export *
      use "Darwin"
      use "_Builtin_stdbool"
      use "_Builtin_stdint"
  }
MODULEMAP

# Add ManisPocketCore.swift to sources if not already present
phase = target.source_build_phase
unless phase.files.any? { |f| f.file_ref && f.file_ref.path == "ManisPocketCore.swift" }
  group = project.main_group.find_subpath("ManisPocket", true)
  phase.add_file_reference(group.new_file("ManisPocketCore.swift"))
end

# Patch all build configurations (Debug + Release)
target.build_configurations.each do |cfg|
  settings = cfg.build_settings

  # Swift include path → module.modulemap lives in ManisPocket/Generated
  paths = Array(settings["SWIFT_INCLUDE_PATHS"] || ["$(inherited)"])
  unless paths.include?("$(SRCROOT)/ManisPocket/Generated")
    paths << "$(SRCROOT)/ManisPocket/Generated"
    settings["SWIFT_INCLUDE_PATHS"] = paths
  end

  # Header search path → ManisPocketCoreFFI.h
  hdrs = Array(settings["HEADER_SEARCH_PATHS"] || ["$(inherited)"])
  unless hdrs.include?("ManisPocket/Generated")
    hdrs << "ManisPocket/Generated"
    settings["HEADER_SEARCH_PATHS"] = hdrs
  end

  # Library search path → target/release (Rust static libs)
  libs = Array(settings["LIBRARY_SEARCH_PATHS"] || ["$(inherited)"])
  unless libs.include?("$(PROJECT_DIR)/target/release")
    libs << "$(PROJECT_DIR)/target/release"
    settings["LIBRARY_SEARCH_PATHS"] = libs
  end
end

project.save
puts "✅ Xcode project patched"
