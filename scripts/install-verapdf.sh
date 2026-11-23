#!/bin/sh
# Install the pinned upstream veraPDF CLI for the container and CI.
set -eu

destination=${1:?absolute installation directory required}
case "$destination" in
    /*) ;;
    *) exit 2 ;;
esac
archival_install_tmp=$(mktemp -d)
trap 'rm -rf "$archival_install_tmp"' EXIT

version=1.30.4
archive=verapdf-greenfield-$version-installer.zip
checksum=04415ef3f149526a3048b9889723e81320ddf013bd891bcc7f0f297f3eadcc23
cd "$archival_install_tmp"
curl --fail --silent --show-error --location --retry 3 \
    "https://software.verapdf.org/rel/1.30/$archive" -o "$archive"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check
unzip -q "$archive"
cat > install.xml <<EOF_XML
<AutomatedInstallation langpack="eng">
    <com.izforge.izpack.panels.htmlhello.HTMLHelloPanel id="welcome"/>
    <com.izforge.izpack.panels.target.TargetPanel id="install_dir">
        <installpath>$destination</installpath>
    </com.izforge.izpack.panels.target.TargetPanel>
    <com.izforge.izpack.panels.packs.PacksPanel id="sdk_pack_select">
        <pack index="0" name="veraPDF GUI" selected="false"/>
        <pack index="1" name="veraPDF CLI" selected="true"/>
        <pack index="2" name="veraPDF Documentation" selected="false"/>
        <pack index="3" name="veraPDF Sample Plugins" selected="false"/>
    </com.izforge.izpack.panels.packs.PacksPanel>
    <com.izforge.izpack.panels.install.InstallPanel id="install"/>
    <com.izforge.izpack.panels.finish.FinishPanel id="finish"/>
</AutomatedInstallation>
EOF_XML
java -jar "verapdf-greenfield-$version/verapdf-izpack-installer-$version.jar" install.xml
"$destination/verapdf" --version
