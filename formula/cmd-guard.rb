# typed: false
# frozen_string_literal: true

# Template — the release workflow substitutes VERSION/SHA placeholders
# and pushes the result to redodson01/homebrew-tap.
class CmdGuard < Formula
  desc "Fast, compiled PreToolUse hook for Claude Code that intercepts dangerous shell commands"
  homepage "https://github.com/redodson01/cmd-guard"
  version "VERSION_PLACEHOLDER"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/redodson01/cmd-guard/releases/download/vVERSION_PLACEHOLDER/cmd-guard-x86_64-apple-darwin.tar.gz"
      sha256 "SHA_MACOS_X86"
    end
    if Hardware::CPU.arm?
      url "https://github.com/redodson01/cmd-guard/releases/download/vVERSION_PLACEHOLDER/cmd-guard-aarch64-apple-darwin.tar.gz"
      sha256 "SHA_MACOS_ARM"
    end
  end

  on_linux do
    if Hardware::CPU.intel? && Hardware::CPU.is_64_bit?
      url "https://github.com/redodson01/cmd-guard/releases/download/vVERSION_PLACEHOLDER/cmd-guard-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA_LINUX_X86"
    end
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/redodson01/cmd-guard/releases/download/vVERSION_PLACEHOLDER/cmd-guard-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "SHA_LINUX_ARM"
    end
  end

  def install
    bin.install "cmd-guard"
  end

  def caveats
    <<~EOS
      To configure the Claude Code hook, run:
        cmd-guard --setup

      Re-run after upgrading to update the symlink.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cmd-guard --version")
  end
end
