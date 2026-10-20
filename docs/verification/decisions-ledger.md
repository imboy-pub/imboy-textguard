# 执行决定

[中文](decisions-ledger.md) | [English](decisions-ledger.en.md)

D001 控制目录映射：用户提示词 CONTROL_ROOT 指向已存在的计划文件，不能把文件当目录或搬移覆盖。匹配原始摘要后采用计划第 1.3/22.1 节固定目录 /Users/leeyi/imboy-evidence/textguard-v2；保留原计划及只读副本，后续同提示词从该稳定目录恢复。该调整属于已授权实际目录映射，不变更 T/AC 义务。

D002 未找到 mem 可执行命令；不臆造 /mem load，不修改本地 memory。当前任务不依赖历史聊天。

D003 T01/AC-111 阶段义务：当前代码证实频道编辑未审核、动态没有编辑路由，App 永久业务拒绝可能入 outbox。T01 为只读盘点任务，不能要求它先完成依赖 T19 的 T20 代码适配；因此 AC-111 按 source_baseline/consumer_final 分阶段，T01 关闭源链与既有定向基线，最终兼容在 T20/T23 关闭。G0 仅使用 baseline 子结果，G8 必须检查 consumer_final，不能把需修改端写成 UNCHANGED_VERIFIED。该解释消除任务卡只读范围与最终 AC 要求的时序矛盾，保留全部验收义务。

D004 业务范围按当前代码冻结：频道内容创建（包括走同一发布函数的 incoming webhook）及授权编辑、动态创建、既有五个资料字段更新；仅在核实获准的公开场景发送明文，私有频道/私有动态必须在远程调用前分流。动态没有现有编辑 API，不额外制造编辑接口；C2C/C2G、密文、评论、注册创建及频道本体字段不因共享底层函数自动纳入。资料现有复审先于保存，后续最小适配需绑定实际写入值与版本；既有普通入队失败可发布但异常边界必须保留并报警，不虚称持久且恰一次复审。

D005 合同 PoC 与产品阶段分开：PoC 仅验证两端生成、presence、稳定应用错误及 deadline，服务明确返回 NOT_READY，不计入引擎或 T19 双协议业务验收。T03 commands 为任务调用接口和预期 oracle 冻结，实际脚本/测试选择器按 T04 起逐项注册，未实现返回 BLOCKED；不把合同列表当作已执行测试。

D006 HTTP/gRPC 错误分类以原始 V2.1 §7.2 为准。旧 Rust 注释和测试把 OVERLOADED 当503、检测预算超限当413、PROTOCOL_ERROR 当参数错误，不能用其作为后端白名单期望。实际红测分别复现503/429、400/503、gRPC INVALID_ARGUMENT/DATA_LOSS。服务和适配器同步修正；HTTP PROTOCOL_ERROR 选502，gRPC选DATA_LOSS，其余内部错误500/INTERNAL。管理CAS412、busy409及原有安全拒绝保持。此为原合同纠错，非降低要求；BACKEND-P1-HTTP-STATUS-CODE沿用累计第二轮预算。

D007 T17 允许路径表未列main/lib/API装配，但实际main仍退出NOT_READY，HTTP/gRPC均绕过已存在Admission；只补runtime文件不能满足跨协议共享准入。集成人按共享文件唯一写权最小扩展装配到main/lib/API和PublishedCore取消回调，保留原协议/Schema及默认运行预算。T17组件独立审查后才集成；真实TLS、故障、隐私最终场景仍归T23，release性能冻结归T18、完整soak归T24，不提前记全AC通过。

D008 T17 入口deadline恢复：旧gRPC组件测试把150ms原样用于core，违反全局80ms预算；实际装配后分别断言20ms客户端截止、150ms被80ms全局预算截断，并证明实际任务未提前释放许可。独立预审进一步指出解码前deadline缺失，真实延迟body反例先红测，再把入口绝对Instant贯穿decode/准入，不在handler重起算。HealthWatch的同状态重复是锁定tonic-health0.12.3实现可见行为（初始borrow后changed），允许重复但1秒最多8条必须观察发布后Serving；没有放松ready必须变化的业务期望。T17-runtime-fix2正式86/86，仅组件phase，不能替代T19/T23/T24。

## T10 regex configuration budget (2026-10-16)

D009 T18真实Linux测量发现自定义hyper接收器绕过tonic默认TCP_NODELAY：grpc单并发20命中响应稳定41—51ms，最后一个高密度请求造成约42ms排空；同一规则纯core约100us。保存T18-perf02原始红证据后，由集成人串行在runtime/server.rs为已接收TCP流启用NODELAY，恢复既有tonic默认。T18卡的benches范围最小扩到这一处接线；协议、匹配、准入、deadline和原性能门槛均不改变。真实LinuxA/B及T17组件组合回归、独立审查后才能集成。基线RSS原失败仍保留，不能因此解释为已修。
Plan §4.7 bounds both compilation and DFA cache and aggregates configuration at 32 MiB; B1 requires all 32 rules. The fixed product builder uses 768 KiB compilation plus 256 KiB DFA per rule, totaling 1 MiB per rule and 32 MiB for 32 rules. No runtime/RSS hard-limit claim. Tests reject inflated compilation, cache, aggregate and expression expansion. AC-041 continuity remains a T14/T22 runtime obligation.

### D010 — T18测量冻结与T24容量验收按phase分开
原计划§13/§15.3及T18卡明示T18测量和事前冻结，T24最终断言。独立审查T18-measurement-review02确认旧runner把短测容量混作早期门禁。保留formal01—03全部失败，新增逐metric严格原阈值观测；T18测量自身的执行/oracle/协议/资源证据门禁不降级。runner仅T18配置phase=T18_measurement，整体AC091/092与G5不置PASS。benches/support-targets.json冻结原50k及增强规模/500完成QPS/P99/RSS，T24预热600s、固定3600s窗口与排空并报，不删高RSS样本、不四舍五入。NODELAY单连接设置失败仅丢连接的最终分支需新镜像实测。独立reviewer_type=agent，不能替代最终容量或业务认可。

### D011 — T19独立黄金数据与冻结例外/CAS合同对齐
attempt09真实118项中7失败；原失败回执保持不变。独立agent使用同一Rust归档binary与合成包执行隔离HTTP诊断，回执T19-attempt09-isolated-diagnostic-UX2sGU/observations.json：alias返回word_source=pattern；仅B授权的例外保留首处T证据；同包旧ETag返回already_active且generation不变。§4.8要求同视图例外，§9.1明确同包幂等优先于不同包CAS。修正测试构造：demo例外显式授权B/T，原局部命中期望保持；新增low的B-only负例证明T不被扩张豁免；CAS同一叶同时证明同包幂等及合法不同包旧ETag拒绝。alias.word在计划未明确canonical/pattern的情况下，冻结为已有Rust返回的配置pattern；独立golden写定合成常量，不调用生产生成期望，rule_id仍绑定canonical，severity/action/偏移断言不变。AC060/077须由新候选正式119项双协议验证及独立delta审查收口；诊断不算验收通过。


## D012 - T20 current-entrypoint binding and public-only transport

Channel public eligibility is visibility=0/access_type=0, active and outside workspace; moment public is visibility=0 (default1=friends). Preserve local legacy checks. Only explicitly configured rust mode sends public text without ciphertext/e2ee/encrypted markers. Unknown channel visibility fails closed. Current approved channel/moment APIs expose content only; profile updates one whitelisted field per operation. No moment edit API is invented.

Channel edit checks after author/window permissions and before workspace_guard transaction; conditional update binds original content+updated_at. Queue only after successful writes; duplicate idempotent creates do not enqueue again. Profile failed saves cannot enqueue. Review removal conditionally matches checked channel/profile content to avoid deleting changed content; workspace guard retained. Metadata JSONB stores structured hits/trace and a content digest. JSON word arrays preserve canonical commas; legacy rows keep fallback display. Current migration maximum194 verified; coordinator allocates195 with up/down.

Applies T20/AC078-083; T21 adds bounded shadow/modes at the same boundary. Final proof requires real isolated PG and Rust; mocks only support targeted fault/notification observations.

## D013 - T20 independent review tightens payload and content-version binding

The first-draft review T20-readonly-review-preflight identified two P1 and two P2 issues: PostgreSQL JSONB payload bytes skipped remote edit checks; caller encryption markers could downgrade public text; content-only review deletion allowed ABA; truncated hit lists lost their completeness flag. This supplements D012 without widening approved scenes. Text payloads accept decoded maps or valid JSON object bytes; malformed or unsupported encryption-marked text is rejected locally with zero remote calls. C2C/C2G are still outside this adapter.

Migration195 additionally stores channel moderation_version and per-profile-field moderation_versions. Initial channel rows use the empty version; every content update rotates a random token, including legacy updates. Profile updates use jsonb_set so changing one field preserves the other field versions. Rust edits compare the content and version observed before detection; newly enqueued channel/profile reviews carry that saved version, and reject uses a conditional version+content match. Old review rows retain their legacy content-only compatibility. Metadata preserves response completeness and snapshot trace without recalculating decision from displayed hits. Verification must exercise real JSONB reads, forged markers, ABA, enqueue exceptions and truncated hits before review closure.

Source caller audit also found generic user_repo updates (for example ai_agent profile nickname). A narrowly scoped PostgreSQL trigger rotates any actually changed approved profile field whose caller did not supply a fresh version; this keeps old review tokens invalid across those existing paths without adding new remote-check scenes. Explicit checked writes keep their supplied token. Trigger creation/removal is part of migration195 up/down and must be tested on the isolated database.

New trigger-generated versions use UUID bytes rendered as 32 lowercase hexadecimal characters (UUID text with hyphens removed), matching checked-write/queue token shape. Reading and comparing an existing opaque stored token remains unrestricted, including a preexisting 36-character UUID; only newly authored schema1 review bindings require the defined 32-byte shape (or an initial empty channel token). This is a producer-format consistency change; the real PG trigger, unchanged-field, checked-write and ABA tests retain their original expectations.

## D014 - T21 shadow uses the actual legacy cache snapshot and bounded admission

Plan §10.1 names conceptual moderation.backend; the current T15/T20 configuration is moderation_detector.mode. Keep that existing key and the approved public plaintext entrypoints. inspect_snapshot uses one cached_words result for all independent fields and computes TGBASEv1 from those exact original word/category/severity tuples. Export reuses the same pure digest encoder. Missing metadata produces legacy_unavailable, never a synthetic comparable allow. Mode shadow retains the old verdict and performs no synchronous Rust call.

The supervised worker uses 64 fixed ETS slots, two linked/monitored workers, and a 1000ms deadline measured at admission. Request bodies never enter the coordinator mailbox. Mode switches change an epoch, discard queued objects, terminate old workers, and release capacity only on DOWN. Legacy/rust remain explicit modes; defaults remain legacy. Fixed classifications distinguish base/normalization/semantics mismatch, comparable decisions, enhanced evidence, unexplained enhanced differences, and facility/drop counters. Enhanced observations are not business approval or compatibility PASS. T21 real PG/Rust HTTP+gRPC verification and independent review remain required; T22 onward is outside the user's current stop boundary.

## D015 T21 gRPC流取消与OTP隐私修复
合同10.1/10.4与T21-review01要求实际底层流清理后才释放影子资源。锁定grpcbox unary超时未取消stream，首轮55叶中53过、2为fixture累计计数错误，cleanup日志另发现任务随机authorization。扩展既有production桥：guardian在建流前获shadow协调器确认；worker与guardian均DOWN才释放并发槽；RST_STREAM CANCEL后用独立monitor确认流DOWN。沿用unary channel pick与send_request；本部署无interceptor，配置未知interceptor明确not_ready，禁止静默忽略。shadow单job结算一次，计数不代表资源已释放。
OTP primary filter在发送者同步执行：精确service标记识别stream终止报告，发送者局部标记继续覆盖proc_lib/supervisor报告族；整体安全event替换，metadata白名单，未知回调/正文/Token不保留；无关sender保持不变。底层解码异常/畸形成功体仍失败。受影响T19共享bridge必须真实双协议回归；Rust核心无变更可复用452组件证据。新增测试与正式freeze02/复审通过前一律未验收。

D015补充（独立delta发现后，正式复测前）：不能把stop后的PID DOWN当作stream-set已释放；RST优先自然关闭，必要时kill后仅关闭本流active登记并核实，无法证明清理则保持guardian槽至连接终结。不能使用库内部默认5秒建流超时丢句柄；改headers-only异步建流，正文留guardian且到期/owner死亡即丢弃，pending连接消息不携正文；保留guardian等待迟到句柄并立即取消。共享supervisor不持久标记。日志判定必须在脱敏之前计数，落盘脱敏继续保留；新增4个正式runner自测验证真实finish门禁对注入输出失败。

## D016 T21 review03 restart ownership and caller deadline
Independent review03 reproduced restart oversubscription, foreground deadline overrun and connection error remapping. The two fixed shadow lease processes now outlive coordinator/guardian death; a registered connection must acknowledge ownership before headers-only stream creation. A lease is released after guardian DOWN with cleanup proof, or actual connection DOWN. Foreground callers use a fixed absolute context deadline and a reply alias; they return timeout without abandoning guardian cleanup or accepting late replies. No foreground two-slot limit is introduced. connection_failed and overloaded retain the existing closed error vocabulary. New real-connection regressions cover restart, 60ms deadline with live caller mailbox, and physical connection failure through bridge/adapter/business. T19 shared bridge acceptance must be rerun after integration. This decision does not advance T23 resource/memory acceptance.

## D017 W5 native correctness environment and generated cache retention
Docker's own host log records an ENOSPC shutdown; shared Docker resources are preserved. W5 correctness uses the installed PostgreSQL18.4 in an isolated loopback cluster with private temporary credentials and the real Rust service. This provides no Linux filesystem, performance, soak or production evidence. Review04 required independent PGDATA/credential directories, preservation when stop/status is uncertain, and PG raw-log scanning before redaction; review05 closes the changes with three real-PG regression tests. Final integration must exercise the updated helper and bind its digest. The failed selftest03 recovery receipt collision remains recorded; selftest04 uses separate recovery receipts without changing fault assertions.

Per the user's disk request, completed task-owned Cargo target/registry copies may be removed after checking process identity, open files, generated-only content and Git ownership. Keep raw logs, receipts, source archives, lockfiles, all recovery branches and final candidate worktrees. Retain the latest useful final build cache; remove superseded generated copies after terminal receipts exist. Cleanup receipt06 records about19.8GiB additional physical recovery; snapshots and original test claims are not rewritten. No shared Docker image/volume, global Cargo cache or user resource is removed.

## 2026-10-18 20:45 session3 (zcode-20261018T2035-resume) 恢复核查结论

1. **锁接管**：前协调器 PID 49711 (zcode-20261018-w6-resume) ps 核实死亡 → 接管；soak09 性能锁 PID 73310 死亡且 T24 已终态 → 归档至 state.backups。
2. **soak09 核验闭环确认**（12:54 已由上一会话落盘，无需重做）：100/100 发布；1h soak-measure 1.8M 请求 p99 3.4ms；AC-095=MECHANISM_VERIFIED+DEFECT_FIXED_PARTIALLY（RSS 缺陷修复 A/B 932MB→222MB；残余 +3.3MB/pub 已文档化；512MiB 未达=594MB；零意外小时被宿主突发阻断）。soak-rss-growth.json 驱动未产出，判定依据 capacity-observations.json + rss-fix-ab，可接受。
3. **imboy 候选漂移澄清（重要）**：用户本人 2026-10-18 17:50 以自身身份将 textguard 任务合并进 imboy main（83899ae2），18:05 合并 PR #20（f3ca4fbf，GitHub noreply）。imboy-integration 分支已随 main 推进至 f3ca4fbf。AC-105 attempt3（20:23 启动）运行于该最新候选上 = 正确的最终候选基础。13:03 版 manifest 记录的 f87aeae2 已过时 → eunit 结束后 T27 冻结时更新。此为用户自主操作，非任务越权合并。
4. **候选完整性误报澄清**：src/textguard.proto 已被 git 跟踪（初次核查 ls-files 输出被 head -15 截断致误判）。src/textguard_pb.erl 为 eunit 启动时 gpb 从 proto 再生的构建产物（Makefile: gpb 生成代码不参与门禁），未跟踪为预期行为。候选干净检出后 make compile 可再生，无缺陷。
5. **G5 补跑决策**：imboy_local（PID 67918，用户进程，Fri 起存活）当前仍 88.7% CPU 突发 + AC-105 eunit 占用 CPU → 本会话不启动容量补跑（性能任务不得与全量 EUnit 并行）。待 eunit 结束后视宿主安静度再评估；不满足则维持 PARTIAL_ENV_BLOCKED 如实交付。

## 2026-10-18 20:52 G5 补跑预案（eunit 完成后执行）

前置条件（全部满足才启动）：(a) AC-105 eunit attempt3 结束并核验；(b) 重启 contention monitor（T24-contention-monitor/，旧 PID 60896 已死）观察 ≥30 分钟 imboy_local <20% CPU 持续；(c) 清理本任务遗留 Docker 容器（t28-*/abA/abB/abC/rebuild），保留 eunit-pg 至 eunit 结束；(d) 性能锁重新登记。

补跑命令（cwd=textguard-integration worktree，镜像 e328a097 最终候选）：
1. `python3 -B tests/performance/run_capacity.py Capacity.test_ac092_b1_capacity` → 目标 B1-grpc 干净窗口（此前 3 次均落争用：formal05 58unexp/122%CPU、formal07 11unexp）
2. `python3 -B tests/performance/run_capacity.py Capacity.test_ac094_boundary_workload` → boundary-4096（此前 4/8/2 unexp 均落突发窗；256 已 PASS）
3. `python3 -B tests/performance/run_capacity.py Capacity.test_ac093_mixed_fairness_and_overload_ladder` → 750/1000/1500 阶梯（fairness 已证，仅缺阶梯）
4. 可选：`Capacity.test_ac095_one_hour_soak_with_100_publications`（零意外小时；RSS 512MiB 界限为已文档化 allocator 限制，重跑不改此子项）

每组之间复查宿主安静度；任一窗口再落争用即停止补跑、保持 PARTIAL_ENV_BLOCKED（证据链已足：formal05/06/07 + monitor timeline 相关性）。imboy_local 为用户进程不得干预。

## 2026-10-19 09:20 AC-105 attempt3 中途诊断（pg_jieba 环境缺口）

**事实**：03:10 CST 出现 1 处失败：adm_organization_handler_tests:setup_db → inttest_marker_db:ensure_extensions → CREATE EXTENSION pg_jieba → 0A000 feature_not_supported。隔离 eunit PG（textguard-eunit-pg-c9ccc9）为原生 postgres:18 镜像，无 pg_jieba/timescaledb/postgis 扩展文件（ls 扩展目录 0 命中）；产品契约（AGENTS/用户环境 imboy_pg18 = imboy/pg18:3.6.1-2）要求这些扩展。attempt1 日志 382744 行有**完全相同**失败（pg_jieba 出现 2 次）→ 该失败与 10-18 晚的候选漂移无关（attempt1 跑在漂移前 f87aeae2），属两次 attempt 共用的环境缺口；上一会话"attempt1 零失败"结论仅检查了 eunit 终局总结标记（attempt1 未跑到终点）。

**定性**：ENV 缺口，非产品缺陷；与仓库既有教义同类（Makefile:95-140 记载的 AG31/SC153 等环境门套件均由专项单跑承载）。

**处置**：不打断 attempt3（13h+ 的已跑部分有效）；等跑完拿 eunit 终局权威失败清单；用 imboy/pg18:3.6.1-2 本地镜像在**同端口 49919** 起替换容器（凭证/库同名，零配置改动）；对每个失败模块 `make eunit t=<模块> EUNIT_CONFIG=config/sys.tg-eunit EUNIT_USE_RELAY=0` 定向重跑取证。预计 cs_application_tests（另一 provision_fresh 用户，未跑）会踩同一缺口，一并处理。

## 2026-10-19 09:40 AC-105 attempt3 adm_admin_handler_tests ×2 失败定性（并发互踩，非回归）

**失败**：13:01:47 / 13:06:05 UTC，config_product_experience_success_payload（期望 configured_value=workspace 实得 chat）与 failsafe_visible（期望 configured_raw=workspce-typo 实得 chat）。

**根因链**（静态取证闭环）：
1. 用户 PR #19（bf5523f9，漂移窗口内用户自有提交）把这两个测试的配置注入从 meck `application:get_env` 改为 `with_product_experience` 助手 = `application:set_env(imboy, product_experience, Value)`（VM 全局状态）。
2. handler `configured_experience()` 读 `application:get_env(imboy, product_experience, chat)`，sys.tg-eunit.config 不设该键（grep=0）。
3. 失败窗口内并发迁移套件以 3 秒/次的节奏令 imboy 应用退出重启（migration_dirty churn，24,828 次全程存在）；每次 app 重启把 app env 重置回 sys.config（未设 → unset → handler 默认 chat）。
4. 两个断言时刻（13:01:47/13:06:05）与 churn 窗口精确重合，签名一致（都读回 chat）。

**定性**：Makefile:95-140 文档化的"独占/全局状态套件在全量 4 路并发轨道必然互踩"类；attempt1 无该模块任何记录（未跑到，非通过）；与 TextGuard 提交无关（TextGuard 不触碰 app env/product_experience 路径）；与候选漂移无关（同窗口用户自己的 PR 引入的新脆弱性，老 meck 设计抗互踩）。

**处置**：不打断套件；完成后与 pg_jieba 环境缺口一并做定向单跑 `make eunit t=adm_admin_handler_tests EUNIT_CONFIG=config/sys.tg-eunit EUNIT_USE_RELAY=0`（修正 PG 49919 = imboy/pg18:3.6.1-2）；单跑绿即证明互踩定性成立。预计剩余运行中还会出现同类的其他套件失败，最终清单统一按"env-gated / app-env-fragile"两类归口定向重跑。

## 2026-10-19 10:40 attempt3 终止根因 + attempt4 设计（分块 + launchd）

**attempt3 终止根因（实锤）**：2026-10-19 09:58:31 CST，beam/make 收 SIGTERM（日志尾 `Terminated: 15` + `SIGTERM received`），同刻：本会话 nohup 派生的争用监控器死亡 + harness 后台任务注册表清空（TaskOutput 报 No task found）。对照 attempt1 终止于 20:20 CST（上一会话收尾快照写于 20:26-20:30，恰在其后）→ **凶手=ZCode harness 会话/后端回收时的进程清理**（重度使用约每 7-14h 发生）。resume.md 声称的 "launchctl 脱离" 未生效——进程仍在可被组杀的范围。

**attempt4 设计**（对上述根因的工程对策）：
1. **launchd 域托管**：`launchctl submit` 实测分离性 OK（测试任务 PPID=1）；supervisor 由 launchd 直接父子，harness 回收不再波及。
2. **分块串行**：1436 模块（966 src + 470 有效套件 − Excl 9）÷ 60/块 = 24 块；逐块独立 VM + 独立 exit/日志凭证；supervisor 幂等（已有 .exit 的块跳过）→ 任何意外终止最多损失当前块，重提交即续跑。
3. **修正 PG**：textguard-eunit-pg-fixed（imboy/pg18:3.6.1-2）已在 49919 服务（见下条）；pg_jieba 环境缺口在块内直接修复，不再依赖事后补跑。
4. EUNIT_MODS 经 MAKEFLAGS 透传内层 make（GNU make 语义），模块集与真实全量轨道对齐（erlang.mk EUNIT_MODS 逻辑复刻 + 项目 Excl 过滤）。

**修正 PG 部署记录（10:25-10:40）**：原生 postgres:18 容器已停（留证不删）；textguard-eunit-pg-fixed = imboy/pg18:3.6.1-2 @ 127.0.0.1:49919，凭证同前（tg_eunit）。绕过镜像 init 双缺陷：10_init.sh 无执行位被 source 时 ${@} 展开为文件列表（chmod +x 修复）；20_update.sh 在临时服务器期建 timescaledb 早于 preload 生效（跳过；postgis 系扩展由运行时 ensure_extensions 按库补建，最终服务器 preload 已生效）。手工建 tg_eunit 库；pg_available_extensions 四扩展齐备；shared_preload_libraries=pgcrypto,pg_jieba,timescaledb,vector,pg_stat_statements。

## 2026-10-19 11:20 adm_admin_handler_tests 单跑：互踩假设推翻，定性改为用户域确定性失败

corrected PG + 无并发环境单跑：**Failed: 2 / Passed: 9**，失败与全量完全同款同签名（line 244 configured_value 期望 workspace 得 chat；line 282 configured_raw 期望 workspce-typo 得 chat）。关键佐证：line 242 effective_product_experience=workspace 断言通过（值来自 meck），line 244 来自真实 application:get_env=未设 → setup 的 set_env 值在断言时不可见。ensure_config_loaded 仅幂等 load 不清 env；确切 wipe 机制未锁定（嵌套 setup 顺序/meck 交互可能），但**分类判定不依赖机制**：单跑确定性复现 ⇒ 非全量互踩；属用户 PR#19 改写后的测试代码在本候选/环境的确定性失败（用户云端 CI 12777 全过=环境差异）。归口 D 类（用户域）：如实报告，不代修用户测试代码，不阻塞 TextGuard 交付。adm_organization_handler_tests 同理修正归类：pg_jieba 缺口已由修正 PG 修复（setup 通过），但测试体 11 处 elib_tsid_not_initialized 单跑复现 → D 类环境敏感。attempt4 在轨清单将给出两模块的一致性证据。

## 2026-10-19 12:00 chunk 环境失败链根因 + 修复（tg_eunit 扩集缺失）

**失败链（实锤）**：chunk-01/定向单跑中 ac04_*/account_ds/adm_organization 出现 elib_tsid_not_initialized（attempt1/3 原生 PG 全量轨道零此失败）→ 链条：修正 PG 的 tg_eunit 全新库无扩展 → foundation 迁移(00000001)调用 create_hypertable（timescaledb）→ 原生 PG 上扩展不可用走条件跳过（成功）；修正 PG 上扩展可用但 tg_eunit 未装扩展 → SQL 42883 create_hypertable does not exist → schema_migrations version=1 dirty=true（02:34:40 写入）→ imboy_migrate:migrate/1:101 每次启动即中止 → imboy app 永不启动 → TSID（imboy_sup:327 于 app 启动初始化）永缺 → 依赖 app 状态的套件连锁失败。eunit_setup 对 app 启动失败静默继续（{app_not_started, test_continues}）放大了损害。

**修复（12:00）**：停 supervisor+进程组（launchctl remove 组杀，未触碰用户 imboy_local）；DROP/CREATE tg_eunit；按 inttest_marker_db ?EXTENSIONS 同款 12 扩展装入库（pgcrypto/pg_jieba/timescaledb/vector/postgis/pg_trgm/btree_gin/btree_gist/citext/unaccent/intarray/pg_stat_statements）；重启 supervisor（PID 70339，PPID=1）。**验证：chunk-01 重跑首启 Application imboy started，migration_dirty=0**。此前 chunk-01 首跑产出为污染证据（日志已被重跑覆盖，无需清理）；两份定向单跑的 elib_tsid 失败同样由该链解释——adm_organization 的 D 类定性需修正为「环境链失败，已修复，待 chunk 重跑定论」；adm_admin 的 2 例修复后待 chunk-01 复核（其 setup 不依赖 provision_fresh，预期仍失败则维持用户域定性）。

## 2026-10-19 12:06 attempt4 首轮 pass 进展与环境稳定确认

chunk-01（修复后重跑）91s EXIT:0 无 eunit 总结 + chunk-02 55s EXIT:2（Failed:6/Passed:444）均含 db_down（03:57-03:58 UTC）= 扩展安装后短暂不稳定窗口（timescaledb workers 初始化/连接风暴），chunk-03 起 env 稳定：EXIT:0 / 377s / 0 failures / db_down 仅 2 处瞬态。**处置**：chunk-01（VM 死亡无总结=不可验证）与 chunk-02（db_down 污染）标记无效，待首轮 pass 至 chunk-24 后删除两者 .exit 重提交自动补跑（supervisor 幂等设计）。有效块证据持续累积。预估全程 ~2.5h（按 chunk-03 均速 6min/块，重块会拉长）。

## 2026-10-19 12:12 attempt4 pass1 新失败集与判别计划

chunks 04/06 出现新失败集（attachment_upload_logic×8、agent_hird_mini_spike×6、attachment_upload_spool×3、cs_first_reply_pg×1、attachment_lifecycle_v5_w403×1 等）；chunks 03/05 干净。attempt3 基线：这些套件全部干净通过。失败签名之一：FK violation fk_ear_a*(23503) + 单例 "Failed to load config /config/sys.config: enoent" 警告（仅 chunk-04 该套件，attempt3 零出现）。三个候选差异变量：(a) 修正 PG 内容（扩展/全新库状态）(b) 分块 VM 冷启动状态 (c) launchd supervisor 环境（PATH 等）。**判别实验**（pass1 结束后执行，避免端口冲突）：对失败套件在交互 shell 跑 `make eunit t=<mod>`（同修正 PG）——绿=supervisor 环境差异（修 supervisor env 后重提交补跑），红=PG/状态差异（进一步二分）。chunk-01/02 维持无效待补跑。

## 2026-10-19 12:55 AC-105 attempt4 终局清单与分类（完整全量证据首次达成）

**历史性事实**：attempt4 完成两次完整 pass（pass1 全 24 块 42 分钟；pass2 补跑 chunk-01/02），本机首次获得完整全量 EUnit 证据。attempt1/3 从未跑到 attachment/enterprise/cs 区域（543 组即死），故"attempt3 这些套件干净"是**无基线假象**（非 verbose 模式通过不打印；死亡无总结）——已修正此前基于它的定性。

**TextGuard 产品套件：9/9 全绿**（textguard_base_digest/business/contract/export/grpc_call/integration/pb/publisher/shadow，chunk-23 Failed:0 Passed:841）。

**adm_admin_handler_tests 复决**：修复环境全量轨道 PASS（pass1/pass2 一致）；其 ×2 失败仅出现于 solo 单跑与 attempt3 原生 PG 上下文 → 上下文依赖型（with_product_experience 的 app env 依赖并发 app 运行时状态），in-track 绿即验收口径绿；上下文敏感性如实记录。adm_organization / ac04_* / account_ds 修复后全绿。

**最终失败分类**（全部非 TextGuard）：
1. 工具链门（设计性 FAIL，缺外部 hird 命令）：agent_hird_mini_spike ×6 + imboy_hird_acceptance ×3
2. macOS /tmp 符号链接类（fixture 未解析符号链接；Linux CI 无此问题；对齐用户 PR"原生验收待完成"自述）：attachment_upload_logic ×8 + attachment_upload_spool ×3 + attachment_lifecycle_v5_w403 ×1；isolated_pg_fixture ×3 为内层 cancel 包装（attachment_pending_location_pg 内层取消，同类域）
3. 显式 env 门（设计性 FAIL）：enterprise_directory_sync_pg ×1（缺 DIRECTORY_SYNC_INTTEST_PG_HOST）
4. 行为单例（机制已录待用户 triage）：eb_seat_send_protocol ×1（500≠200）、cs_first_reply_pg ×1（42501 trg_enterprise_message_purge_guard）、enterprise_internal_audit_policy_pg ×1、enterprise_internal_wiring_http ×1
5. 取消：chunk-08/16/20/23 各少量（Failed:0；已知全量轨道现象，主分支历史 62 cancel 先例）

## 2026-10-19 13:10 终判落盘 + G5 窗口开启

**AC-105/G8 终判**（详见 state）：attempt4 两次完整 pass = 本机首次完整全量证据。TextGuard 9/9 绿；修复环境 adm_* 全绿；残余失败全部非 TextGuard 且分类完备（native-runner 门 22 + 行为单例 4 + 少量 cancel）。native-eunit-run.sh 为 GitHub-runner-only 设计（本机 exit 2 硬校验），门环境本机不可复现系用户设计；用户自述"cloud 12777 passed / native pending"互证。**总状态 = PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS**（计划 1319 严格口径：无关失败未清不得冠以 LOCAL_CANDIDATE_PASS；TextGuard 全部范围 AC 均绿）。

**G5 补跑启动**（13:07）：用户已重启 imboy_local（旧爆冲 PID 67918 死，新 PID 68476 安静），窗口三天来最佳；ac092-b1-grpc → ac094-boundary → ac093-ladder 顺序补跑（launchd textguard-g5，最终镜像 sha256:e328a097...，per-group evidence dir，争用监控重定向 68476）。驱动器三处修正：完整 sha256 镜像引用/独立 ev 目录（exist_ok=False）/docker PATH（launchd 无 Docker Desktop 路径）。

## 2026-10-19 15:45 会话收尾（session3 终）

G5 终态：ac092 PASS / ac094 PASS / ac093 PARTIAL_NARROWED（阶梯 1000 点 6 处意外=0.001%、1500 未达；fairness 两度证明）。清理完成：本任务容器全删、eunit-pg 两容器删、/tmp 脚本删、launchd 标签清零；证据目录全保留；用户资源未动。终态总览见 state.json（overall=PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS；TextGuard 全范围绿）。

## 2026-10-19 imboy-textguard 主目录切换到 main
- 应用户质询"为什么 imboy-textguard/ 没有提交"：产品代码 73 个提交一直都在仓库（main=integration=49dfe3f），但主目录停在 bootstrap 占位分支（仅 .gitignore+README.md，工作区干净），因 git 同一分支不可同时在两个 worktree checkout，main 之前放在证据区 worktree。
- 处置：注销 textguard-main worktree（工作区干净，分支与提交不受影响，runs/ 证据不动），主目录 git checkout main，现 HEAD=49dfe3f，完整产品代码可见。
- 同日：a2fe60cd 全量 EUnit 重跑 24/24 chunks 完成（T27-eunit-attempt4 收据全部为今日），残差与已分类清单一致，TextGuard 9/9 绿；ac093 第二次复现后按 PARTIAL_NARROWED_REPRODUCED 定案。

## 2026-10-19 aho-corasick 依赖升级至 1.1.5
- 用户在 imboy-textguard 主目录手改 Cargo.toml 钉死 =1.1.5（原提交态 =1.1.3），并指认上游 BurntSushi/aho-corasick tags 存在 1.1.5。
- 取证：crates.io 已发布 1.1.5（2026-08-03，未 yank，与上游 tag 同日）；本地 AC 镜像仓 tag 5178060=1.1.5，master 领先 2 个 lint 提交，镜像不参与依赖解析（lock 走 registry）。
- 处置：cargo update -p aho-corasick --precise 1.1.5 刷新 lock（仅此包，checksum c982642f...）；cargo check --locked --all-targets 通过；以 leeyi 身份提交 Cargo.toml+Cargo.lock 到 main（新 HEAD 见 git log）。

## 2026-10-19 任务 worktree 磁盘清理
- 用户授权清理。删除 textguard-v2/worktrees 下全部 33 个 worktree（28 个干净直删；5 个仅含未跟踪文件的先核验：observability 与 textguard_pb 未跟踪副本=已提交版本或有既有保留；shadow 测试 3 个草稿与已提交版有差异，连同 sys.tg-eunit.config 备份至 runs/.../pre-cleanup-untracked/ 后 force 删除）。
- 6.4G→12K；全部分支与提交保留在 4 个主仓；prune 顺带清掉 imboy 仓内 /private/tmp/imboy-unified-main-merge-20261013 失效注册（目录早已不存在，仅登记清理）。
- 未动：.codex-runs/.codex 下其他任务线 worktree；runs/ 21G 取证；Docker 镜像（信息项已报给用户）。

## 2026-10-19 证据区构建缓存清理
- 删除 runs/.../ 下 8 个纯 cargo 构建缓存（5×rust-target 带 CACHEDIR.TAG + 2×cargo-home registry + contract-poc/target），共约 4.5G，均可由已提交源码+lock 再生，无证据价值；textguard-v2 由 21G 降至当前值。
- 待用户定夺的候选第二层：被后续轮次取代的中间尝试（T21-shadow-attempt03/04/05/08/09 各 304M、T24-soak08 395M、早期 formal/ac096 轮次），需与 state.json 判定引用交叉核对后删。
- 必留：T24-g5-rerun 2.1G（AC-093 复现证据，根因排查要用）、formal/soak09-fixed 终版、T27 全量 EUnit、T28、release/state/resume/decisions。

## 2026-10-19 二层清理（用户授权）
- 删除无 state 引用的中间目录 ~400M + Docker 任务卷21个/旧镜像8个tag/builder容器/构建缓存1.1G。收口资产清单核对 OK（state/resume/decisions/release/g5-rerun/eunit-attempt4/T28-run07/formal05/soak09-fixed/pre-cleanup-untracked）。

## 2026-10-19 Docker 剩余可回收清理（用户授权，删除清单存档）
### 删除的 dangling 卷（66 个）
0e4221d7074f6b5686952794f7426ba87d5960a78036209bc9bd372404367c8f
02f979a5092cdf01f3a93ad576fa3112f68136fa195014e39535c1aa032fe9b3
3b59f9def9fc9ddd75ac31bb0ecb2ac6ed7fa11edb63c72996edc066cdd74b2a
4a597035d30bc717201facce43533a44f45b4ceda0804b179fd2e250cf598458
4c38a7e0c68d12ea439d0d3e0eb842250f9c5456679d39c9730a0c4fb89ee140
4c463ee92f7d2151f1bd6fc78ed27fbee2b3afaeba815e711a37c0f249ceb11b
4e05fbf38323512c1abb44592455468b3366bdd52ae97edbbf46d29167059d82
5df16e52ed8cc0a88e65d7cc41b26cab29615e7681c110c82dd47a8326d78efe
5ee6e4742a76ce5e2d4a86567fff76b56fe4ba585dc2ca09dda96ce4e14e1742
5f0530a3edf423b89e9e05a0345981ee5a4d76d0de97c3cf5e806840226aee3b
6b04f610f8e7272a44c18bc98b159bfb65424035722d65bd064a83e264b81f8a
7fdca538605c9dc013a13a5fe85ddae2a3d7c3b43fbc53708af31289e7d402ac
8a0f868616310f88aea5c1b9fe0cf6e951ee6ae730f3ab897c6da0242e239fff
8e8b853a780b4419216b50eda9e104c9c80b18b982c27f6eeb4869b1171c8a16
11c777e46ee19fda0e56dd699b0209feb3c5f11a3630122bdb062894c9a1bf0f
23b8de3b5002ee1c1aded97de46c620ad2c8295760600349b3a2c951742d5f70
27d37c9c8d854400cc823ec85003e7752de35fc8fd663bf3d85ce84cd0ccc2b2
30a5e11d5e0c42e21f21f932545701a6908f83ebc9727a6a550d624e2899f9f9
31e1266fe235003beb707e283b769fa129d3fff5a31c6e5b7f31dc32e04d250d
39b32aa3cd70f7d7a854292b4a15efc6659a1c04908a04d9d255a66cf01fa433
81e3583398d0d1ed8e4ef4aa4731d252570de66c658282d0884a8fa3bcd66eda
97f9679f5a328d64a18cc6427559d6c9c79d33c96f24afc6f96dc7a64143d8b1
230caf9eb9ed8c81d19ea291a88f3cb6e14306255288f77b0dcb382483520e8c
441b43bce961b76697e3fd737d0027e0361e989d27ecbcf8921fbf5a30d37134
469db9fa3b10b25ba526a56d0185fbe12d7aea1361a566110d1eacd9674fe9e1
511f0c2aa55c9488af0cad2ae657d8f2891e868bfc92e522e9174e6d1c7732e5
667eab6a911dd0b8c1af0a678852e19af79d72c4ac12c8e007a6c0676c563d3e
857b86dbeb3483392c03a65532ebae5ca8887e30939ea1c854deefa7ce766847
1953d31a2f25b742f818a100e5c2d1c7fa27267b2df98ff404dd3b08ca180607
4583f6d9c9889cbe9877efc050b9e548951a11474f5005ce60b4bb2faf13e2ca
10467a1b37ac99eb0c0d4e5ff7d3fbb982b9bf43a2b8aa7247631fc74e20e52a
52118e05be05f8f06021c55a06fef50f25ef41a82b6ef0f5cafaabf8988c1deb
61313dae035bcd5b641dda4bff9f8690dc561c36dc1a12d09c4223b7325c07cb
73518b0179fc8b54c54c6dca4b73f2b7ef8f2a93f008590bd877c2a33263ea08
727546cf61ece22f2b195928c1148c2c23711ec8e8c3ace0e38629a17ee1a71c
7824429fe5682e19bc672d511f1d93da6d83370457b44e3a94e632dbb54313e5
4343712382b86e553b2630608fbe8470968c7d91db48c5bd1b5877de2ae230c9
b9c5725f35c028516618e92ef21b729eb74ba926453ddf2ffa2797217057a412
b49ffe2253aeafc3a1498c8bdcf1f3baec01eafacdd6ea59cb2782af26e2a2d1
b925f66a3704cd9712e75e40bd5b4ba715652336daed34abe850820375e6503a
bb11f55945ce6ec16e78de47eee5a0b4941eecbd1e996d2f2badc1caa8c9965c
bdaf811e4f87b4911e102b9857506d68641219971911ab91ae1ece07d79305e0
buildx_buildkit_mybuilder0_state
d5e0519764da199ee78016c119469fe7afc71ec7e401cf459ecf428a8fb5599b
dacdbe8636e385d25b7b7587eaa65b41ad23a9eed4d35e783e4ca7207a3befc1
dcfe22cef1e5808f97eda9e156c352516b4f574f96bd22a886809d8628c3ab30
de965229363ceba9ddafb34e5e0ce600099860b84d994334fdf6d33d2bd9a864
deepflux-e2e-garage
deepflux-poc_garage-data
deepflux-poc_redis-data
deploy_fuyao-redis-data
dev_garage
dev_nats
dev_pg
dev_qdrant
e1b4d6f41c9eb34f34f8518680957e3efd7ff0266d7ddff068676c41eadafa81
e71e6ad5f839e8ef35b96bf7e2c51b402586e0474bd07d512ce5c59dec20118c
e9676f0acc95246ca079f168a86b768511f9663b730c4e128033bf69c3fb3eac
edd6dd088fe06007191165dd2296d4b7a542dc9392a100152de4d7561c3da451
f5d568ec57cfa650403134f9c164fd47ed9b1bebc31baa1704beab36d2792d80
f85e893cfc631b3b98dbcb610d478589d402d8984d3f76808ba956188cc23a95
infra_redis_data
infra_rmqdata
tg-run-20261016t075111z-pg-data
tg-run-20261016t075111z-pg-r1-data
tg-run-20261016t075111z-toolchain

### 删除的未使用标准镜像
imboy-backend:tsid-v10-hcfix imboy-backend:tsid-v10-3a01829d mysql:8 postgres:18 docker:dind oven/bun:1-slim livekit/livekit-server:v1.13.7 livekit/livekit-cli:v2.1.1 dxflrs/garage:v2.3.0 dxflrs/garage:v2.0.0 nginx:alpine nginx:1.27-alpine redis:7-alpine nats:2.10-alpine alpine:latest debian:trixie-slim prom/alertmanager:latest
### 保护未动
imboy/pg18:3.6.1-2（两个运行容器在用）、textguard-perf:t24fix-01、textguard-builder-base:1、pgvector/pgvector:pg18（收口）、csww-widget:v0/v1、cs-hosted-widget-img:20260920t150315（自定义构建、无法重拉、共150M）
### 执行结果
- 卷 68→2（仅存运行容器挂载），回收 ~9.2G；镜像 28→11（6.29G→3.71G），回收 ~2.6G（125 条 untagged/deleted）；宿主机可用 53G。
- 剩余"可回收" 2.718G 均为刻意保护项：textguard-perf:t24fix-01 + builder-base + pgvector（收口）+ csww 自定义构建（无法重拉）。

## 2026-10-19 21:10 G5 安静窗口复跑终判（session4 zcode-20261019-1930-resume）

- **环境变化**：争用源 imboy_local（PID 68476）死亡（19:27 核实 ps 空），宿主仅剩本会话与 GUI；新监控 timeline3.jsonl 全程见证。前协调器 codex-20261019T170219（PID 72561）2.5h 零 CPU、无 flock、无状态写入，按先例接管锁。
- **rerun3**（19:40–20:53，4370s，launchd com.imboy.textguard.g5r3，主仓 runner + 镜像 sha256:e328a097 t24fix-01）：ac092 `ok`（warm/cap http 0-fail p99 4.1–4.6ms；**cap-B1-grpc 0-fail p99 3.9ms**——历史 3 次争用失败点位干净复现，与 T18 基线 3.7ms 一致；mixed p99 3.9–4.4ms；closed-B1 13.8k QPS）。ac093 `ok`：**750/1000 档 unexpected=0 rejected=0 failed=0；1500 档 unexpected=0**（唯一 1 failed 为 HTTP_429 显式准入拒绝，断言允许的有界拒绝）。两次复现的窄发现（1000 档 6/9 处 ~130ms 截止超时、与 83–115% imboy_local 尖峰精确同窗）在安静窗口完全消解，争用归因闭环。
- **ac094-clean**（20:56–21:02，345s）：256/4096/16384/multi32k/dense 五档 unexpected=0 全绿（14:20 争用窗同款 EXIT:0 的干净重证）。
- **终判**：G5=PASS_WITH_DECLARED_SOAK_RESIDUAL；AC-092/093/094=PASS；AC-095 维持 PARTIAL_DEFECT_FIXED（rss_growth_bound +3.3MB/pub 为分配器高水位残余，A/B 修复证据 932→222MB 已文档化，16.4 解释路径满足；重跑不改变定性故不重跑 soak；其 unexpected=49 已由 timeline 归因首 5 分钟争用突发，5–60 分钟为 0）。AC-105 收口为 PASS_WITH_DECLARED_RESIDUALS（补齐 T27 任务级定案的 AC 级落盘）。T24=COMPLETED_DECLARED_RESIDUALS。
- **顺带修复**：磁盘 state 中 AC-091/096 的目录型 evidence 引用与 G1/G3/G4/G6/G7 空引用（17:00 会话绕过校验直写所致）已改为具体文件并补齐 sha256，checkpoint.save 全量校验恢复通过。
- overall 维持 PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS（计划 15.5：G8 全量 EUnit 存在与本任务无关的既有失败——native-runner 门 22 处为 GitHub-runner-only 设计 + 4 处既有行为单例，授权范围外）。

## 2026-10-19 21:57 目标完成审计 + U 线条件项收口（session4）

- 审计发现 U 线簿记缺口：UC-004—015 与 UG0—UG2 仍为 NOT_RUN，而 U01 已定案 NO_UPSTREAM_CHANGE_NEEDED。按计划 15.3"U线未激活项记NOT_APPLICABLE"修正：UC-004/005 及 UC-007—015 → NOT_APPLICABLE（条件未激活）；UC-006 → PASS（其判据恰为"明确无候选"这一已发生结论，证据=T26 独立审查+追溯）；UG0 → PASS（资格门实际执行并合法终止候选线：U00 PASS + U01 无通用价值）；UG1/UG2 → NOT_APPLICABLE；UG3/UC-016—018 维持 NOT_AUTHORIZED。
- 终态扫描：tasks 36/36、acceptance 120/120、gates 10/10、upstream_acceptance 18/18、upstream_gates 4/4 全部终态，无 NOT_RUN/IN_FLIGHT/PENDING/BLOCKED/FAIL。
- 计划内任务在当前授权下的完成定义已达成：所有可执行任务均有真实终态与有效证据；T29/U05/UG3/UC-016—018 按提示词第 1.2/13.8 节记 NOT_AUTHORIZED（公开投稿与生产为另行授权项，不是未完成任务）。overall 维持 PARTIAL_COMPLETE_NOT_LOCAL_CANDIDATE_PASS，其与 LOCAL_CANDIDATE_PASS 的差距全部由授权外既有失败构成（native-runner 门设计 + 4 处用户域行为单例），解锁条件已写入 resume.md。

## 2026-10-20 05:40 独立复核（review）发现与修复（session4，只读审计+簿记修复）

- **修复 1（本会话引入）**：AC-092/093/094/G5 引用的 timeline3.jsonl 摘要过期——G5 收口落盘时监控进程仍在追加写，哈希随即失效。已按当前文件字节刷新。
- **修复 2（历史遗留）**：79 处 PASS 项证据引用从未记录 sha256（早期会话只记路径；checkpoint 校验器只查存在性故未拦截）。已全部回填。
- **修复 3（本会话引入）**：build_manifest 的 git() 辅助函数 .strip() 导致 cargo_lock_sha256 按去换行字节计算。已改为精确 blob 字节；修正后 5e07482a... 与 2026-10-18 原始 manifest 绑定值一致（交叉印证候选 lock 未变更）。
- **复验结果**：391 处 PASS 项证据引用全部存在、摘要全部匹配；checkpoint 全量校验通过；manifest 与 git 分支引用（head/tree）逐项一致。
- 判定层面无变更：所有状态、门禁、残余声明维持原样。

## 2026-10-20 05:5x 第二轮 review：manifest 引用哈希次序缺陷修复

- AC-104/106/119/120 以 release-manifest.json 为哈希证据；上轮"先回填摘要、后修 lock 口径重生成 manifest"的次序使这 4 处变旧。已刷新。
- 规则（已入 resume 检查单）：任何 manifest 重生成后，必须重刷以其为证据的引用哈希；聚合文档做哈希证据时其后不可原地重写，只能换新引用。

## 2026-10-20 06:30 第四轮 review：运行时启动性冒烟（零缺陷）

- 候选 49dfe3f 冷 worktree 按 README 命令构建 21.2s 通过、contract 测试 7/7、真实启动六项行为探针全过（401/400/503-NOT_READY-绝不allow/401/503+200/gRPC可达）；发布库防符号链接加固意外获得运行时实证（/tmp 符号链接祖先被拒）。
- 无状态变更；收据见 runs/run-20261016T075111Z/R4-startability-smoke.md。评审资源（worktree/secrets）已清理。

## 2026-10-20 07:0x 第五轮 review：功能闭环冒烟（零缺陷）

- 真实二进制发布→检测闭环全通：CAS 发布/ready 翻转/快照身份拒配/七项检测探针全部合同正确（含决策-动作-严重度分离、原文字节偏移、视图域例外、双语义场景）。例外"未抑制"初判为探针误解——例外按视图豁免（B-only 时 T 见证存活）是 §4.8 合同行为，B+T 声明即 allow，已实证。
- 三端漂移审计：app/admin 自验证头以来零审核合同路径变更。
- 收据 runs/run-20261016T075111Z/R5-functional-e2e-smoke.md；资源已清理，无状态变更。

## 2026-10-20 07:3x 第六轮 review：gRPC 线缆 + §9.4 恢复冒烟（零缺陷）

- grpc_contract 15/15（真实 TCP）；发布后重启词库存活（revision/instance 一致、检测有效）；ACTIVE 损坏后 fail-closed（503 NOT_READY，不崩溃不误放行）。收据 R6-recovery-grpc-smoke.md；资源已清理。

## 2026-10-20 08:0x 第七轮 review：imboy 产品侧 TextGuard 漂移审计（零缺陷）

- f3ca4fbf→a2fe60cd 唯一 TextGuard 变化 = 生成件 textguard_pb.erl 入库（proto 零变化）。提交版与 Makefile 规范版布局不同但导出函数集一致、maps 表示一致、CheckTextRequest 编码字节级一致 → 功能零漂移，EUnit 证据实质覆盖候选头（AC-118 漂移审计产品侧闭合）。卫生备注：建议用户经 make 再生 pb 以字节规范跟踪（用户域，非阻断）。
- verify_textguard.sh --list 无环境时 BLOCKED/exit2 = §15.3 fail-safe 设计 ✓。
- 收据 R7-imboy-drift-audit.md；无状态变更。

## 2026-10-20 08:3x 第八轮 review：代码卫生审计（零缺陷）

- Rust src/ 无 dbg!/TODO/DIAG；println/eprintln 仅版本输出与启动失败/panic 可观测性；历史 DIAG 残留已由 11dea3e 清除。Erlang moderation 路径零残留。CI 与 deploy/README 声明（端口/凭证/80ms 预算/loopback metrics）与代码逐项一致。收据 R8；无状态变更。

## 2026-10-20 09:0x 第九轮 review：并发+CAS 冒烟、操作手册整合（零缺陷）

- 60 并发检测全 200 判定正确；CAS 三向厘清：陈旧+同内容=幂等 200（无害）、陈旧+不同内容=412、非法包=400 悬挂引用拒绝。整合 OPERATIONS-RUNBOOK.md 补齐 §11.3 命令清单。收据 R9；无状态变更。

## 2026-10-20 09:3x 用户授权的合并确认、分支清理与测试数据清理

- **合并确认**：三端无需实际合并——textguard 仓 25 个任务分支与 imboy 仓 10 个任务分支经 `git branch --no-merged main` 核验全部已合并（历史会话中用户已自主合入 main）；无任务相关未提交改动（imboy 5 项、imboyadmin 1 项脏文件均为用户 WIP，未代提交）。
- **迁移文件（数据库增长文件）核验**：imboy main 唯一迁移根 priv/migrations 共 197 迁移，.up.sql 编号无重复；textguard 00000197_review_metadata 与用户 00000198_conversation_last_msg_id_xid 顺序衔接无冲突。
- **可追溯性**：删分支前打 tag——textguard 仓 `textguard-candidate-run-20261016T075111Z`→49dfe3f、imboy 仓 `imboy-textguard-candidate-run-20261016T075111Z`→a2fe60cd；release-manifest 的 branch 字段自此为历史名，SHA 经 main 血缘与 tag 均可解析。
- **分支删除**：安全 `-d` 删除 35 个任务分支，0 拒绝（全部确认已合并）；本任务 worktree 已全部不存在（未触碰其他任务线的 .codex-runs worktree）。
- **测试数据清理**：docker 删 textguard-perf:t24fix-01（122MB）与 textguard-builder-base:1（427MB）；/tmp 清 tg-review-secrets、boot 日志、detach 测试输出、tg16-enh-corrupt.json（损坏注入产物）；用户 PG 中零本任务测试库（eunit 用隔离容器已删）；未动：imboy_pg18 容器与库、其他任务线的 inttest_*/imboy_closeout_* 测试库、pgvector:pg18 镜像（归属存疑留用户定夺）。
- 未 push（未获授权）。

## 2026-10-20 生产部署执行（T29/G9，用户明确授权）

- 授权：用户明确"授权部署到 prod，二进制模式"。服务器 106.53.76.53:32（pro-imboy，Debian12 x86_64）。
- 执行：Mac Docker 交叉构建（rust:1.90-bookworm+protoc，TEXTGUARD_BUILD_ID=0.1.0-prod_49dfe3f，二进制 sha256 b622639762b3…）→ scp 校验后安装至 /usr/local/imboy-textguard-0.1.0-prod_49dfe3f/bin → systemd imboy-textguard.service（专用用户 imboy-tg，NoNewPrivileges/ProtectSystem 加固，仅绑 loopback 8081/8082/8083）→ 发布空规则集 generation 1（遵循 x-textguard-confirm-empty 防呆头；部署排错点：secret 属主未改导致 SECRET_FILE_UNREADABLE，chown 后恢复——fail-closed 如设计工作）。
- 验证：live 200 / ready 200（gen1）/ 检测 allow+complete / 未认证 401 / systemd enabled 开机自启。
- 边界：Erlang 后端保持 mode=legacy 不切流（业务零影响）；峰值观察与业务切流待后续授权/流程。操作手册见 docs/operations/deploy-binary-prod.md（4b27a43）。
- 同日用户清理：仓外证据区（17G 原始数据）按用户决定删除，本目录为唯一证明载体。
