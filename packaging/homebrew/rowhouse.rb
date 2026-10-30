# Homebrew cask for Rowhouse. Filled in by `node scripts/packaging.mjs <version>`;
# publish it in a tap repo (github.com/giankbr/homebrew-tap → Casks/rowhouse.rb),
# then: brew install --cask giankbr/tap/rowhouse
cask "rowhouse" do
  version "__VERSION__"
  sha256 "__SHA256_DMG__"

  url "https://github.com/giankbr/rowhouse/releases/download/v#{version}/Rowhouse_#{version}_universal.dmg"
  name "Rowhouse"
  desc "Compact, open-source database client with an MCP server"
  homepage "https://giankbr.github.io/rowhouse/"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on macos: ">= :catalina"

  app "Rowhouse.app"

  zap trash: [
    "~/Library/Application Support/id.novagen.rowhouse",
    "~/Library/Caches/id.novagen.rowhouse",
    "~/Library/WebKit/id.novagen.rowhouse",
  ]
end
