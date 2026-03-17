class Kabel < Formula
  desc "Multi-agent communication CLI for Claude Code and OpenCode"
  homepage "https://github.com/LarryStanley/kabel"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/LarryStanley/kabel/releases/latest/download/kabel-aarch64-apple-darwin.tar.gz"
      # sha256 will be filled after first release
    end

    on_intel do
      url "https://github.com/LarryStanley/kabel/releases/latest/download/kabel-x86_64-apple-darwin.tar.gz"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/LarryStanley/kabel/releases/latest/download/kabel-aarch64-unknown-linux-gnu.tar.gz"
    end

    on_intel do
      url "https://github.com/LarryStanley/kabel/releases/latest/download/kabel-x86_64-unknown-linux-gnu.tar.gz"
    end
  end

  def install
    bin.install "kabel"
  end

  test do
    assert_match "kabel", shell_output("#{bin}/kabel --version")
  end
end
