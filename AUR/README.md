# foldcli AUR package: first upload guide

These files prepare the Arch Linux package recipe. Nothing here has been
uploaded. The recipe targets version `0.1.1` and downloads the `v0.1.1` Git tag.
The package includes the GPL-3 license. Publish the project files and the
matching `v0.1.1` tag before building/uploading.

## 1. Publish the upstream release

Commit and push the project to GitHub, then create and push the release tag
that matches `pkgver` in `PKGBUILD`:

```sh
git tag v0.1.1
git push origin v0.1.1
```

If the version is already tagged, don't recreate it. Confirm the tag points to
the source containing the package code and GPL-3 license.

## 2. Install build tools on Arch Linux

```sh
sudo pacman -S --needed base-devel git rust
```

## 3. Create an AUR account and add your SSH key

Sign in or register at <https://aur.archlinux.org/> using the username
`space.ixc`. Add the public SSH key from your computer in your AUR account
settings. If you do not have a key, create one with `ssh-keygen -t ed25519`,
then add the contents of `~/.ssh/id_ed25519.pub` to AUR. Never share the private
key (`~/.ssh/id_ed25519`).

Check that SSH authentication works:

```sh
ssh -T aur@aur.archlinux.org
```

## 4. Build and install the package locally first

From the project root, generate package metadata and build the package:

```sh
cd AUR
makepkg --printsrcinfo > .SRCINFO
makepkg -si
```

The first command creates the metadata AUR requires. The second downloads the
tagged source, builds it, and asks pacman to install it. Check that `foldcli`
starts before uploading.

## 5. Upload the package recipe to AUR

The first upload uses an empty AUR Git repository. Clone it, copy the package
files into it, then commit and push:

```sh
cd ..
git clone ssh://aur@aur.archlinux.org/foldcli.git foldcli-aur
cp AUR/PKGBUILD AUR/.SRCINFO foldcli-aur/
cd foldcli-aur
git add PKGBUILD .SRCINFO
git commit -m "Initial upload: foldcli 0.1.1"
git push
```

After the push completes, visit <https://aur.archlinux.org/packages/foldcli>.
For later updates, edit the recipe, regenerate `.SRCINFO`, test with `makepkg`,
then commit and push the updated files from `foldcli-aur`.

Arch's submission guide explains the package repository and metadata workflow:
<https://wiki.archlinux.org/title/AUR_submission_guidelines>.
