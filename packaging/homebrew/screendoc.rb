class Screendoc < Formula
  desc "Record your terminal and publish it as GIF, MP4, asciinema cast, and web player"
  homepage "https://github.com/GlobalTechInfo/screendoc"
  # sha256 placeholders are backfilled from the release artifact checksums
  # (release assets ship `<artifact>.sha256`) after the first tagged release.
  version "0.1.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-apple-darwin.tar.gz"
      sha256 "TODO"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-apple-darwin.tar.gz"
      sha256 "TODO"
    end
  end

  on_linux do
    if Hardware::CPU.aarch64?
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "TODO"
    else
      url "https://github.com/GlobalTechInfo/screendoc/releases/download/v#{version}/screendoc-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "TODO"
    end
  end

  def install
    bin.install "screendoc"
  end

  test do
    system "#{bin}/screendoc", "--version"
  end
end
