#!/bin/sh

VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
git tag "v$VERSION"
git push origin "v$VERSION"
