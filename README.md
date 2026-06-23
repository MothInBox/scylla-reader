
# Scylla Reader
A TUI reader that interfaces with .WASM plugins to allow for an easily extensible Reader and Library manager.



## Layout

The project  contains multiple crates:

* **`scylla-reader/`**: The primary Terminal User Interface application.
* **`scylla-plugin-api/`**: Definitioins for plugin developers.
* **`plugin-template/`**: A baseline implementation used for learning to make new scrapers with Extism.

## Installation

### Option A: Using Nix (Recommended)

#### Temporary:
Compile and run once immediently:
```bash
nix run github:MothInBox/scylla-reader
```
Comile and add to temporary shells path:
```bash
nix shell github:MothInBox/scylla-reader
```
#### Declarative Installation (flake.nix and home.nix)
Update flake.nix:
```nix
{
    inputs = {
        nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";
        home-manager = {
            url = "github:nix-community/home-manager/release-26.05";
            inputs.nixpkgs.follows = "nixpkgs";
        };
        # Add Scylla Reader to your flake inputs
        scylla-reader = {
            url = "github:MothInBox/scylla-reader";
            inputs.nixpkgs.follows = "nixpkgs";
        };
    };

    outputs = { self, nixpkgs, home-manager, scylla-reader, ... }: { # Add to outputs too!
        nixosConfigurations."your-username" = nixpkgs.lib.nixosSystem {
            modules = [
                {nixpkgs.hostPlatform = "x86_64-linux";}
                home-manager.nixosModules.home-manager
                ({ config, pkgs, lib, ... }: {
                    home-manager = {
                        #Anything else you need here
                        # Pass flake inputs downstream to your home.nix file
                        extraSpecialArgs = { inherit scylla-reader; };
                    };
                    #Anything else you need here
                })
            ];
        };
    };
}
```
Then in home.nix add to your home packages
```nix
{ config, pkgs, inputs, ... }: {
  home.packages = [
    scylla-reader.packages.${pkgs.system}.default
  ];
}
```

### Option B: Using Cargo (UNTESTED)

### Prerequisites
must have the Rust stable toolchain installed on your system along with native development headers for ssl and curl.

Ubuntu/Debian: ``` sudo apt install build-essential pkg-config libssl-dev libcurl4-openssl-dev ```

Fedora: ```sudo dnf groupinstall "Development Tools" && sudo dnf install pkg-config openssl-dev libcurl-devel ```

Arch Linux: ``` sudo pacman -S base-devel pkg-config openssl curl ```

macOS: ``` brew install openssl curl pkg-config ```

#### Compilation
```bash
git clone https://github.com/MothInBox/scylla-reader.git
cd scylla-reader/scylla-reader
cargo install --path scylla-reader

# if you then want to install the template plugin:
cd plugin-template
make
```
The template plugin can then run by this command seqeunce in the TUI
```
i (open add book window)
type "template" 
ctrl + s (submit all)i
```

## Where can I get plugins?
As of right now, develop your own or find one someone else has developed!
``
> [!WARNING]
> Be aware that plugins could be running anything on your machine. Verify the plugin yourself or go with trusted sources.`
