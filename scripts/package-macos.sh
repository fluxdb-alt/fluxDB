#!/usr/bin/env bash
set -euo pipefail

APP_NAME="${APP_NAME:-FluxDB}"
PRODUCT_NAME="${PRODUCT_NAME:-FluxDB}"
BUNDLE_IDENTIFIER="${BUNDLE_IDENTIFIER:-com.shining3d.fluxdb}"
PROFILE="${PROFILE:-release}"
DIST_DIR="${DIST_DIR:-target/macos-package}"
CREATE_DMG="${CREATE_DMG:-1}"
AD_HOC_SIGN="${AD_HOC_SIGN:-1}"

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_CRATE="$WORKSPACE_ROOT/apps/fluxdb-desktop"
TARGET_DIR="${CARGO_TARGET_DIR:-$WORKSPACE_ROOT/target}"
VERSION="${VERSION:-$(awk -F '"' '/^version = / { print $2; exit }' "$DESKTOP_CRATE/Cargo.toml")}"

cd "$WORKSPACE_ROOT"

if [[ "$PROFILE" == "release" ]]; then
    cargo build --release -p fluxdb-desktop
else
    cargo build -p fluxdb-desktop
fi

BINARY_PATH="$TARGET_DIR/$PROFILE/fluxdb-desktop"
if [[ ! -x "$BINARY_PATH" ]]; then
    echo "Missing built binary: $BINARY_PATH" >&2
    exit 1
fi

PACKAGE_ROOT="$WORKSPACE_ROOT/$DIST_DIR"
APP_DIR="$PACKAGE_ROOT/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
INFO_PLIST="$CONTENTS_DIR/Info.plist"
ICON_TIFF_PATH="$PACKAGE_ROOT/AppIcon.tiff"
ICON_PATH="$RESOURCES_DIR/AppIcon.icns"
DMG_PATH="$PACKAGE_ROOT/$APP_NAME-$VERSION.dmg"
ZIP_PATH="$PACKAGE_ROOT/$APP_NAME-$VERSION.zip"

rm -rf "$APP_DIR" "$ICON_TIFF_PATH" "$DMG_PATH" "$ZIP_PATH"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

install -m 755 "$BINARY_PATH" "$MACOS_DIR/fluxdb-desktop"
cp -R "$DESKTOP_CRATE/assets" "$RESOURCES_DIR/assets"

sips -s format tiff "$DESKTOP_CRATE/assets/app-icon.png" --out "$ICON_TIFF_PATH" >/dev/null
tiff2icns "$ICON_TIFF_PATH" "$ICON_PATH"
rm -f "$ICON_TIFF_PATH"

# ---- 内嵌非系统动态库（如 Homebrew openssl@3）----
# 构建产物可能以绝对路径（/opt/homebrew、/usr/local）链接第三方 dylib，
# 在未装 Homebrew 的 Mac 上会 dyld 报错无法启动。这里把这类依赖拷入
# Contents/Frameworks，并把所有引用改写为 @executable_path 相对路径，
# 形成自包含 bundle。安装名修改会使原签名失效，因此逐库重签（ad-hoc），
# 主程序与 bundle 由后面的 codesign 步骤统一重签。
FRAMEWORKS_DIR="$CONTENTS_DIR/Frameworks"
mkdir -p "$FRAMEWORKS_DIR"

bundle_external_libs() {
    local main_bin="$1"
    # targets 兼作待扫描队列与已拷入 dylib 列表；索引 i 前进避免数组切片（bash 3.2 + set -u 兼容）
    local -a targets=("$main_bin")
    local -a copied=()
    local i=0
    while (( i < ${#targets[@]} )); do
        local target="${targets[i]}"
        i=$((i + 1))
        local dep
        # otool -L 第一行为文件名头，跳过；取安装名列并去掉尾部版本说明
        while IFS= read -r dep; do
            case "$dep" in
                /opt/homebrew/*|/usr/local/*) ;;
                *) continue ;;
            esac
            local name
            name="$(basename "$dep")"
            local new_ref="@executable_path/../Frameworks/$name"
            local dest="$FRAMEWORKS_DIR/$name"
            if [[ ! -e "$dest" ]]; then
                # 解析 Homebrew 符号链接到 Cellar 内真实文件再拷贝
                local real="$dep"
                while [[ -L "$real" ]]; do
                    local link
                    link="$(readlink "$real")"
                    case "$link" in
                        /*) real="$link" ;;
                        *) real="$(dirname "$real")/$link" ;;
                    esac
                done
                cp "$real" "$dest"
                chmod u+w "$dest"
                install_name_tool -id "$new_ref" "$dest"
                targets+=("$dest")
                copied+=("$dest")
            fi
            install_name_tool -change "$dep" "$new_ref" "$target"
        done < <(otool -L "$target" | awk 'NR > 1 { sub(/ \(.*$/, "", $1); print $1 }')
    done
    local lib
    for lib in ${copied[@]+"${copied[@]}"}; do
        codesign --force --sign - "$lib"
    done
    if [[ ${#copied[@]} -gt 0 ]]; then
        echo "Bundled external libs: ${copied[*]}"
    fi
}

bundle_external_libs "$MACOS_DIR/fluxdb-desktop"

plutil -create xml1 "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleDevelopmentRegion string en" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string $PRODUCT_NAME" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleExecutable string fluxdb-desktop" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string AppIcon" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string $BUNDLE_IDENTIFIER" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleInfoDictionaryVersion string 6.0" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleName string $PRODUCT_NAME" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundlePackageType string APPL" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleShortVersionString string $VERSION" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :CFBundleVersion string $VERSION" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :LSApplicationCategoryType string public.app-category.developer-tools" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :NSHighResolutionCapable bool true" "$INFO_PLIST"
# TCC 用途声明：日志路径可能被用户设置为 ~/Downloads 等受保护目录，
# 缺少对应 UsageDescription 时系统会静默拒绝（EPERM）而非弹窗授权
/usr/libexec/PlistBuddy -c "Add :NSDownloadsFolderUsageDescription string 'FluxDB 需要将日志写入您选择的下载目录'" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :NSDocumentsFolderUsageDescription string 'FluxDB 需要将日志或数据写入您选择的文档目录'" "$INFO_PLIST"
/usr/libexec/PlistBuddy -c "Add :NSDesktopFolderUsageDescription string 'FluxDB 可能需要访问您选择的桌面目录用于导出文件'" "$INFO_PLIST"

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$APP_DIR"
elif [[ "$AD_HOC_SIGN" == "1" ]]; then
    codesign --force --sign - "$APP_DIR"
fi

if [[ "$CREATE_DMG" == "1" ]]; then
    # 标准 DMG 布局：卷内包含 app 本体 + 指向 /Applications 的软链（Finder 识别为 alias），
    # 打开 DMG 后即可直接把 app 拖到 Applications 图标完成安装
    STAGING_DIR="$PACKAGE_ROOT/dmg-staging"
    rm -rf "$STAGING_DIR"
    mkdir -p "$STAGING_DIR"
    # ditto 保留权限与代码签名（codesign 签名内嵌于 Mach-O，复制不会失效）
    ditto "$APP_DIR" "$STAGING_DIR/$APP_NAME.app"
    ln -s /Applications "$STAGING_DIR/Applications"
    if hdiutil create -volname "$APP_NAME" -srcfolder "$STAGING_DIR" -ov -format UDZO "$DMG_PATH" >/dev/null; then
        echo "Created $DMG_PATH"
    else
        echo "hdiutil failed; creating zip package instead." >&2
        ditto -c -k --keepParent "$APP_DIR" "$ZIP_PATH"
        echo "Created $ZIP_PATH"
    fi
    rm -rf "$STAGING_DIR"
fi

echo "Created $APP_DIR"
