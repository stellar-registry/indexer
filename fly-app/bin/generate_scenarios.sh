#!/usr/bin/env bash
cd scenarios_generator
nix --extra-experimental-features nix-command --extra-experimental-features flakes run > ../src/generated.rs