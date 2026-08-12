class Seal < Formula
  desc "Cross-platform secrets manager (CLI + GUI) backed by the OS keychain"
  homepage "https://github.com/bucabay/seal"
  url "https://github.com/bucabay/seal/archive/refs/tags/v0.1.1.tar.gz"
  sha256 "9d4727818214319b78693cc4fc0500e1e555f72d93bf7f242b7ce091ea94ef3f"
  license "MIT"
  head "https://github.com/bucabay/seal.git", branch: "main"

  depends_on "rust" => :build
  depends_on "node" => :build
  depends_on "pnpm" => :build

  on_linux do
    depends_on "dbus"
    depends_on "libsecret"
  end

  def install
    # Build frontend (required by tauri::generate_context! at compile time)
    system "pnpm", "install"
    system "pnpm", "build"

    # Build the Rust binary
    system "cargo", "build", "--manifest-path", "src-tauri/Cargo.toml", "--release"

    # Install the `seal` binary
    bin.install "src-tauri/target/release/seal"

    # Install the agent skill
    (share/"seal/skills/seal").install "skills/seal/SKILL.md"

    # Install GUI app bundle on macOS
    if OS.mac?
      app_path = "src-tauri/target/release/bundle/macos/Seal.app"
      prefix.install app_path if File.exist?(app_path)
    end
  end

  def caveats
    <<~EOS
      To enable the Seal agent skill (Claude Code / opencode):
        ln -s "#{opt_share}/seal/skills/seal" "$HOME/.claude/skills/seal"
    EOS
  end

  test do
    system "#{bin}/seal", "--help"
    assert_predicate share/"seal/skills/seal/SKILL.md", :exist?
  end
end
