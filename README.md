# TokenBar

菜单栏里看 Cursor、Claude 和 OpenAI 的用量。托盘默认是余额条（满条表示剩余多），没有数据的服务不显示。

Menu-bar usage for Cursor, Claude, and OpenAI.

## 构建与运行

```bash
cargo build -p tokenbar --release
./target/release/tokenbar
```

`tokenbar --check` 打印一份 JSON 后退出。Linux 编译前安装 `packaging/linux/apt.txt` 里的依赖。

## 许可

[Apache-2.0](LICENSE)

## 下载

[Releases](https://github.com/xinxiuzhu/TokenBar/releases)
