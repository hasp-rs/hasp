-- Package directory states.
CREATE TABLE packages.directory_states (
  state TEXT PRIMARY KEY NOT NULL
);
INSERT INTO packages.directory_states VALUES ("not-installed");
INSERT INTO packages.directory_states VALUES ("installing");
INSERT INTO packages.directory_states VALUES ("installed");

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
  state TEXT NOT NULL REFERENCES directory_states(state),

  -- namespace + name + hash should be unique
  UNIQUE(namespace, name, hash)
);

-- Packages being installed.
CREATE TABLE packages.installing (
  installing_id INTEGER PRIMARY KEY,
  -- The id in packages.directories.
  directory_id INTEGER NOT NULL REFERENCES directories(directory_id),
  -- The method of installation.
  install_method TEXT NOT NULL,
  -- Whether this is a force installation.
  force BOOLEAN NOT NULL,
  -- The start time of the installation.
  start_time DATETIME NOT NULL,
  -- The path to which the installation is being performed, relative to the path in the directory id.
  new_dir TEXT NOT NULL,
  -- The path to which an existing installation will temporarily be moved to.
  old_dir TEXT NOT NULL,
  -- Extra metadata associated with the install method.
  metadata TEXT NOT NULL
);
CREATE INDEX packages.installing_directory_id ON installing (directory_id);

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

-- Binaries.
CREATE TABLE packages.binaries (
  binary_id INTEGER PRIMARY KEY,
  -- The name of the binary.
  name TEXT NOT NULL,
  -- The installation the binary is associated with.
  install_id INTEGER NOT NULL REFERENCES installed(install_id)
);
CREATE INDEX packages.binaries_install_id ON binaries (install_id);
