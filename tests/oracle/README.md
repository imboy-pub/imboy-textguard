# T05 独立 oracle

[中文](README.md) | [English](README.en.md)

运行 `./scripts/verify_textguard.sh --task T05 --evidence <new-directory>`。
该通道校验独立参考与 fixture。生产候选比对仍属 T08/T11；这不是完整的 AC-013 产品验收。

`unicode16.py` 只读取 `tests/fixtures/oracle/unicode16/` 下经 SHA 校验的入库表。它使用
UnicodeData 的简单小写映射、无条件的 SpecialCasing 全映射，以及带完整 Cased/Case_Ignorable
区间的默认 Final_Sigma。它从不调用宿主 Unicode 的大小写或属性、Rust，或其他生产实现。
每个原始标量拥有它所产生的每个输出标量的完整字节跨度。

每个 body、canonical 规则、别名与例外都显式给出语义。`compat-v1` 保持冻结的 OTP 无上下文
映射（ΟΣ → οσ）；`enhanced-v1` 应用默认上下文（ΟΣ → ος）。宽度折叠与指定的零宽删除发生在
大小写处理之后。局部例外是独立的朴素包含参考，按 scene/rule/variant/view 限定作用域，
仅对增强的字面量/别名候选启用，排除 gap。正则不在 T05 范围内。

官方输入 URL、SHA-256 值与派生表哈希固定在 `unicode16/manifest.json` 中；Unicode 许可证随表
一同分发。该许可证是 2026-10-16 获取的官方当前文本，而不是声称的 2024 历史快照。用官方文件
离线重抽取：

```sh
python3 -B tests/oracle/extract_unicode16.py --source-dir <ucd-inputs> --check
```

或使用 `--download --check` 获取精确的官方输入并校验全部哈希。这些显式来源审计命令会访问
网络；普通 oracle 测试为离线运行，不需要原始的 3 MB UCD 文件。抽取器在哈希变化或遇到不
支持的默认大小写上下文时失败。

16 个归一化锚点为手工指定，包括 U+1C89、新字母的 Final_Sigma 上下文、两种语义、扩展与字节
来源。变异会强制出错误的 body/规则/别名/例外语义，或移除新增的 Cased 属性，必须被检出。
原有的删除/位移/伪造命中变异保持。原始 seed-20261016 的 1 万语料与 manifest 保持字节一致；
重新生成仅在内存中与冻结 SHA 比对，测试永不写入。
