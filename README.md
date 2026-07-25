# Floaty

桌面便签式 todo 工具。每张便签是独立浮动窗口，支持 markdown、智能时间识别、到期通知，完全本地。macOS + Windows。

[![release](https://img.shields.io/github/v/release/cn-qlg/floaty?display_name=tag)](https://github.com/cn-qlg/floaty/releases/latest)

<p align="center">
  <a href="https://github.com/cn-qlg/floaty/releases/latest">下载最新版</a> ·
  <a href="docs/USER_GUIDE.md"><strong>使用手册</strong></a> ·
  <a href="#开发">开发</a> ·
  <a href="#路线图">路线图</a>
</p>

---

## 功能一瞥

- **多便签浮动窗口**：拖动 / resize / 单独置顶 / 菜单栏常驻（纯 tray 应用）
- **窗口绑定（仅 macOS）**：把便签挂到指定窗口实例，随其全屏 Space 显示；只读取窗口元数据，不请求屏幕录制权限或捕获窗口内容
- **Markdown 编辑**：checkbox / 列表 / 引用 / 代码块 / 粗斜删除线 / 链接 …… 完整双向 round-trip
- **选中文字 + 空行浮动工具栏**：不会 markdown 也能插入待办、加粗、建标题
- **智能时间识别**：输入 `今天下午3点` / `30分钟后` / `下周一` / `4月25日` 等直接识别 → 彩色 pill + 系统通知
- **紧急度配色**：逾期红 / 1h 红 / 今天橙 / 本周黄 / 更晚灰，每分钟刷新
- **外观自定义**：每便签独立背景色 / 透明度 / 字号 / 字色
- **跨平台快捷键**：全局 `⌘⇧N` / `Ctrl+Shift+N` + 一套便签内操作键
- **完全本地**：SQLite 单文件 · 无网络依赖 · 无账号

---

## 安装

从 [Releases 页面](https://github.com/cn-qlg/floaty/releases/latest) 下载对应平台的包。

### macOS（Apple Silicon / M 系列）

```bash
# 打开 dmg 把 Floaty.app 拖到 Applications
# 首次启动前去除隔离（没有代码签名）：
xattr -dr com.apple.quarantine /Applications/Floaty.app
open /Applications/Floaty.app
```

或右键 App → **打开** → 再点一次 **打开**。

### Windows（x64）

运行 `.msi` 安装器。首次启动 SmartScreen 警告 → **More info** → **Run anyway**（因为没代码签名）。

---

## 怎么用？

👉 **[完整使用手册](docs/USER_GUIDE.md)** —— 从新手上手到所有快捷键、时间识别、外观定制、常见问题全覆盖。

安装后首次启动会自动弹一张**上手指南便签**，内容按你的操作系统自动适配。老用户随时可从菜单栏 **📖 上手指南** 再打开。

---

## 技术栈

Tauri 2 · React 19 · TypeScript · Vite · TipTap 3 · Tailwind CSS · sqlx (SQLite) · Tokio · Vitest · Rust

项目包含前后端自动化测试。macOS/Windows 打包走 GitHub Actions 自动构建；窗口绑定是 macOS 专属功能。

---

## 开发

```bash
git clone https://github.com/cn-qlg/floaty.git
cd floaty
npm install

# 开发
npm run tauri dev           # 启动 app
npm test                    # 前端测试
cd src-tauri && cargo test --lib && cd ..   # 后端测试
npx tsc --noEmit            # 类型检查

# 本地打包
npm run tauri build         # 输出 .dmg 或 .msi 到 src-tauri/target/release/bundle/
```

**数据位置：**
- macOS: `~/Library/Application Support/app.floaty.desktop/floaty.db`
- Windows: `%APPDATA%\app.floaty.desktop\floaty.db`

### 发布新版本

```bash
git tag v0.1.2
git push origin v0.1.2
# GitHub Actions 自动跑 Windows + macOS 构建，上传到 Release
```

---

## 设计文档

- [Spec](docs/superpowers/specs/2026-04-17-floaty-design.md)
- [Phase 1 — 脚手架 + 单便签](docs/superpowers/plans/2026-04-17-floaty-phase-1-foundation.md)
- [Phase 2 — 多便签 + 菜单栏](docs/superpowers/plans/2026-04-17-floaty-phase-2-multi-sticky.md)
- [Phase 3 — 视觉自定义](docs/superpowers/plans/2026-04-17-floaty-phase-3-customization.md)
- [Phase 4 — 时间紧急度](docs/superpowers/plans/2026-04-17-floaty-phase-4-due-times.md)
- [Phase 5 — 提醒系统](docs/superpowers/plans/2026-04-17-floaty-phase-5-reminders.md)
- [Phase 6 — 智能时间解析](docs/superpowers/plans/2026-04-20-floaty-phase-6-smart-time.md)

---

## 路线图

- [ ] Snooze / 完成 pill 右键菜单（后端已 ready）
- [ ] 提前 X 分钟提醒
- [ ] 快捷键自定义（真 · 键绑定）
- [ ] 导出 / 导入 markdown
- [ ] 自定义背景图
- [ ] Universal macOS binary（含 Intel）
- [ ] Linux AppImage
- [ ] iCloud / Git 同步

---

## License

MIT
