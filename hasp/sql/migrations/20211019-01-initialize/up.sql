-- Allowed namespaces.
CREATE TABLE packages.namespaces (
  namespace TEXT PRIMARY KEY NOT NULL
);
INSERT INTO packages.namespaces VALUES ("cargo");

-- Package directories.
CREATE TABLE packages.directories (
  directory_id INTEGER PRIMARY KEY,
  -- The namespace for this package.
  namespace TEXT NOT NULL REFERENCES namespaces(namespace),
  -- The name of the package.
  name TEXT NOT NULL,
  -- A hash for the directory.
  hash BLOB NOT NULL,
  -- The package version.
  version TEXT NOT NULL,
  -- Metadata associated with the package as a JSON blob (includes features, source etc).
  metadata TEXT NOT NULL,
  -- The current state (not-installed, installing, installed)
  installed BOOLEAN NOT NULL,

  -- namespace + name + hash should be unique
  UNIQUE(namespace, name, hash)
);

-- Packages currently installed.
CREATE TABLE packages.installed (
  install_id INTEGER PRIMARY KEY,
  -- The id in packages.directories.
  directory_id INTEGER NOT NULL REFERENCES directories(directory_id),
  -- The time at which the installation was completed.
  install_time DATETIME NOT NULL,
  -- Metadata associated with the install as a JSON blob.
  metadata TEXT NOT NULL
);
CREATE INDEX packages.installed_directory_id ON installed (directory_id);

-- Installed files.
CREATE TABLE packages.installed_files (
  installed_file_id INTEGER PRIMARY KEY,
  -- The installation the binary is associated with.
  install_id INTEGER NOT NULL REFERENCES installed(install_id),
  -- The name of the installed file.
  name TEXT NOT NULL,
  -- The hash of the installed file.
  hash BLOB NOT NULL,
  -- Metadata associated with the installed file.
  metadata BLOB NOT NULL,
  -- Whether this file is a binary for which a shim will be created.
  is_binary BOOLEAN NOT NULL,

  -- Each install ID should have unique file names.
  UNIQUE (install_id, name)
);
CREATE INDEX packages.installed_files_name ON installed_files (name);
