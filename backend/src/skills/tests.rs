use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

    use super::*;

    #[test]
    fn additional_tools_and_name_based_settings() {
        let base = temp("additional"); let h = homes_at(&base); let mut cfg=Config::default();
        for id in ["gemini","opencode","qoder","trae","zcode","workbuddy"] { assert!(spec(id).is_some(), "{id}"); }
        for id in ["gemini", "opencode"] {
            let r=write_root(&h,&cfg,id,Scope::Global,None).unwrap();
            let dir=skill(&r.path,"demo",&body("demo"));
            set_enabled_impl(&h,&mut cfg,&r,&dir,"demo",false,&base.join("data")).unwrap();
            let (items,_)=collect(&h,&cfg,spec(id).unwrap(),&mut vec![]); assert!(!items[0].enabled);
            set_enabled_impl(&h,&mut cfg,&r,&dir,"demo",true,&base.join("data")).unwrap();
            let (items,_)=collect(&h,&cfg,spec(id).unwrap(),&mut vec![]); assert!(items[0].enabled);
        }
        assert!(write_root(&h,&cfg,"workbuddy",Scope::Global,None).is_err());
    }

    #[test]
    fn opencode_project_inherits_global_permission() {
        let base=temp("opencode-inherit");let h=homes_at(&base);let cfg=Config::default();let project=base.join("project");fs::create_dir_all(&project).unwrap();
        let spec=spec("opencode").unwrap();let global=write_root(&h,&cfg,"opencode",Scope::Global,None).unwrap();
        set_json_skill(&h,spec,&global,"demo",false,&base.join("data")).unwrap();
        let local=write_root(&h,&cfg,"opencode",Scope::Project,Some(&project)).unwrap();
        assert!(json_skill_off(&h,spec,&local,"demo").unwrap());
        assert!(set_json_skill(&h,spec,&local,"demo",true,&base.join("data")).is_err());
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jolu-skills-{tag}-{}-{}", std::process::id(), stamp()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn homes_at(base: &Path) -> Homes {
        homes_from(base.to_path_buf(), |_| None)
    }

    fn claude_home(h: &Homes) -> PathBuf {
        h.env["CLAUDE_CONFIG_DIR"].clone()
    }

    fn codex_home(h: &Homes) -> PathBuf {
        h.env["CODEX_HOME"].clone()
    }

    fn skill(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SKILL_FILE), body).unwrap();
        dir
    }

    fn body(name: &str) -> String {
        format!("---\nname: {name}\ndescription: {name} 描述\n---\n")
    }

    fn fmt_roots(list: &[Root]) -> Vec<String> {
        list.iter()
            .map(|r| format!("{}|{:?}|{}|{}|{}|{:?}|{}", r.platform, r.scope, r.project.as_ref().map(|p| p.display().to_string()).unwrap_or_default(), r.path.display(), r.readonly, r.skip, r.parked))
            .collect()
    }

    /// 重构前 `roots()` 的输出（2026-10-23 用旧实现跑出的固定值）
    fn roots_before() -> Vec<String> {
        [
            r#"claude|Global||C:\h\cc\skills|false|["synced"]|false"#,
            r"claude|Synced||C:\h\cc\skills\synced|true|[]|false",
            r"claude|Project|D:\p1|D:\p1\.claude\skills|false|[]|false",
            r"claude|Project|D:\p2|D:\p2\.claude\skills|false|[]|false",
            r"codex|Global||C:\h\.agents\skills|false|[]|false",
            r#"codex|Global||C:\h\cx\skills|false|[".system"]|false"#,
            r"codex|System||C:\h\cx\skills\.system|true|[]|false",
            r"codex|Project|D:\p1|D:\p1\.agents\skills|false|[]|false",
            r"codex|Project|D:\p1|D:\p1\.codex\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.agents\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.codex\skills|false|[]|false",
        ]
        .map(String::from)
        .to_vec()
    }

    #[cfg(windows)]
    #[test]
    fn specs_preserve_claude_codex_roots() {
        let h = homes_from(PathBuf::from(r"C:\h"), |name| match name {
            "CLAUDE_CONFIG_DIR" => Some(r"C:\h\cc".into()),
            "CODEX_HOME" => Some(r"C:\h\cx".into()),
            _ => None,
        });
        let cfg = Config::default();
        let projects = [PathBuf::from(r"D:\p1"), PathBuf::from(r"D:\p2")];
        let mut got = fmt_roots(&roots(&h, &cfg, "claude", &projects));
        got.extend(fmt_roots(&roots(&h, &cfg, "codex", &projects)));
        assert_eq!(got, roots_before());
        // 没设环境变量时用默认位置
        let d = homes_from(PathBuf::from(r"C:\h"), |_| None);
        assert_eq!((claude_home(&d), codex_home(&d)), (PathBuf::from(r"C:\h\.claude"), PathBuf::from(r"C:\h\.codex")));
        let mut got = fmt_roots(&roots(&d, &cfg, "claude", &projects));
        got.extend(fmt_roots(&roots(&d, &cfg, "codex", &projects)));
        // 按旧实现推导：Homes { claude: user/.claude, codex: user/.codex, user }
        let expected_default = [
            r#"claude|Global||C:\h\.claude\skills|false|["synced"]|false"#,
            r"claude|Synced||C:\h\.claude\skills\synced|true|[]|false",
            r"claude|Project|D:\p1|D:\p1\.claude\skills|false|[]|false",
            r"claude|Project|D:\p2|D:\p2\.claude\skills|false|[]|false",
            r"codex|Global||C:\h\.agents\skills|false|[]|false",
            r#"codex|Global||C:\h\.codex\skills|false|[".system"]|false"#,
            r"codex|System||C:\h\.codex\skills\.system|true|[]|false",
            r"codex|Project|D:\p1|D:\p1\.agents\skills|false|[]|false",
            r"codex|Project|D:\p1|D:\p1\.codex\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.agents\skills|false|[]|false",
            r"codex|Project|D:\p2|D:\p2\.codex\skills|false|[]|false",
        ];
        assert_eq!(got, expected_default);
        // 隔离环境：按旧实现推导 base = <JOLU_TEST_DIR>/skills-home，claude = base/.claude，codex = base/.codex，不读环境变量
        let iso = isolated_homes(Path::new(r"D:\t"));
        assert_eq!(iso.user, PathBuf::from(r"D:\t\skills-home"));
        assert_eq!((claude_home(&iso), codex_home(&iso)), (PathBuf::from(r"D:\t\skills-home\.claude"), PathBuf::from(r"D:\t\skills-home\.codex")));
        let mut got = fmt_roots(&roots(&iso, &cfg, "claude", &[]));
        got.extend(fmt_roots(&roots(&iso, &cfg, "codex", &[])));
        let expected_iso = [
            r#"claude|Global||D:\t\skills-home\.claude\skills|false|["synced"]|false"#,
            r"claude|Synced||D:\t\skills-home\.claude\skills\synced|true|[]|false",
            r"codex|Global||D:\t\skills-home\.agents\skills|false|[]|false",
            r#"codex|Global||D:\t\skills-home\.codex\skills|false|[".system"]|false"#,
            r"codex|System||D:\t\skills-home\.codex\skills\.system|true|[]|false",
        ];
        assert_eq!(got, expected_iso);
        // 启停方式与校验规则
        let (claude, codex) = (spec("claude").unwrap(), spec("codex").unwrap());
        assert_eq!((claude.toggle, claude.require_name, claude.writable), (Toggle::Park, false, true));
        assert_eq!((codex.toggle, codex.require_name, codex.writable), (Toggle::CodexToml, true, true));
        assert!(spec("grok").is_none() && roots(&h, &cfg, "grok", &projects).is_empty());
        // 约定：home_env 控制 global 的最后一项，且这一项以 `/<子目录>` 结尾
        for s in SPECS {
            if s.home_env.is_some() {
                assert!(env_default(s).is_some(), "{} 的 global 最后一项应以 home_env 的子目录结尾", s.id);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn codex_agents_dir_ignores_codex_home() {
        let h = homes_from(PathBuf::from(r"C:\u"), |name| (name == "CODEX_HOME").then(|| r"E:\codex-home".into()));
        let cfg = Config::default();
        let globals: Vec<_> = roots(&h, &cfg, "codex", &[]).into_iter().filter(|r| r.scope == Scope::Global).map(|r| r.path).collect();
        assert_eq!(globals, vec![PathBuf::from(r"C:\u\.agents\skills"), PathBuf::from(r"E:\codex-home\skills")]);
        assert_eq!(codex_config_path(&h, spec("codex").unwrap()), PathBuf::from(r"E:\codex-home\config.toml"));
        // 新建 / 导入到全局仍写 ~/.agents/skills
        assert_eq!(write_root(&h, &cfg, "codex", Scope::Global, None).unwrap().path, PathBuf::from(r"C:\u\.agents\skills"));
        // Claude 不受 CODEX_HOME 影响
        assert_eq!(claude_home(&h), PathBuf::from(r"C:\u\.claude"));
    }

    #[test]
    fn global_override_replaces_default() {
        let base = temp("override");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let custom = base.join("my-skills");
        skill(&custom, "custom-one", &body("custom-one"));
        skill(&claude_home(&h).join("skills"), "default-one", &body("default-one"));

        let missing = base.join("missing").display().to_string();
        assert!(set_global_dir_with(&h, &mut cfg, "claude", Some(&missing)).unwrap_err().contains("目录不存在"));
        assert!(set_global_dir_with(&h, &mut cfg, "claude", Some("relative/dir")).unwrap_err().contains("目录不存在"));
        assert!(set_global_dir_with(&h, &mut cfg, "grok", Some(&custom.display().to_string())).is_err());
        assert!(cfg.global_override.is_empty());

        set_global_dir_with(&h, &mut cfg, "claude", Some(&custom.display().to_string())).unwrap();
        let list = roots(&h, &cfg, "claude", &[]);
        assert_eq!((list[0].scope, &list[0].path), (Scope::Global, &custom));
        assert_eq!((list[1].scope, &list[1].path), (Scope::Synced, &custom.join("synced")));
        let o = overview_with(&h, &cfg, "claude").unwrap();
        assert_eq!(o.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["custom-one"], "只扫描新目录");
        let info = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(info.overridable && info.overridden);
        assert_eq!(info.default_path.as_deref(), Some(claude_home(&h).join("skills").display().to_string().as_str()));
        assert_eq!(o.platforms.iter().find(|p| p.id == "claude").unwrap().global_dir.as_deref(), Some(custom.display().to_string().as_str()));
        // 新建写到新目录；工具配置（settings.json）位置不变
        let input = SaveInput { platform: "claude".into(), scope: Scope::Global, project: None, name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert!(Path::new(&save_with(&h, &mut cfg, input, &data).unwrap().path).starts_with(&custom));
        assert!(locate(&h, &cfg, &custom.join("new-one").display().to_string()).is_ok());
        // 存进 skills.json
        save_config(&data, &cfg).unwrap();
        assert_eq!(load_config(&data).global_override.get("claude"), Some(&custom.display().to_string()));

        // 恢复默认；选回默认目录也等同恢复默认
        set_global_dir_with(&h, &mut cfg, "claude", None).unwrap();
        assert_eq!(roots(&h, &cfg, "claude", &[])[0].path, claude_home(&h).join("skills"));
        set_global_dir_with(&h, &mut cfg, "claude", Some(&claude_home(&h).join("skills").display().to_string())).unwrap();
        assert!(cfg.global_override.is_empty());
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let info = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(info.overridable && !info.overridden && info.default_path.is_none());

        // Codex：更换的是 $CODEX_HOME/skills，~/.agents/skills 照常扫描、仍是写入目标，且不能被选作替换目录
        let agents = base.join(".agents").join("skills");
        fs::create_dir_all(&agents).unwrap();
        assert!(set_global_dir_with(&h, &mut cfg, "codex", Some(&agents.display().to_string())).unwrap_err().contains("已经是"));
        set_global_dir_with(&h, &mut cfg, "codex", Some(&custom.display().to_string())).unwrap();
        let paths: Vec<_> = roots(&h, &cfg, "codex", &[]).into_iter().map(|r| (r.scope, r.path)).collect();
        assert_eq!(paths, vec![(Scope::Global, agents.clone()), (Scope::Global, custom.clone()), (Scope::System, custom.join(".system"))]);
        assert_eq!(write_root(&h, &cfg, "codex", Scope::Global, None).unwrap().path, agents);
        let o = overview_with(&h, &cfg, "codex").unwrap();
        let flags: Vec<_> = o.roots.iter().filter(|r| r.scope == Scope::Global).map(|r| (r.overridable, r.overridden)).collect();
        assert_eq!(flags, vec![(false, false), (true, true)]);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn global_override_rejects_other_tools_and_project_dirs() {
        let base = temp("override-others");
        let h = homes_at(&base);
        let s = |p: &Path| p.display().to_string();
        let project = base.join("proj");
        let project_claude = project.join(".claude").join("skills");
        let agents = base.join(".agents").join("skills");
        let codex_skills = codex_home(&h).join("skills");
        let claude_skills = claude_home(&h).join("skills");
        let custom = base.join("my-skills");
        for d in [&project_claude, &agents, &codex_skills, &claude_skills.join("synced"), &custom] {
            fs::create_dir_all(d).unwrap();
        }
        let mut cfg = Config { projects: vec![s(&project)], ..Default::default() };
        let err = |cfg: &mut Config, platform: &str, path: String| set_global_dir_with(&h, cfg, platform, Some(&path)).unwrap_err();

        // 其他工具的全局目录（两个方向）
        let e = err(&mut cfg, "claude", s(&agents));
        assert!(e.contains("已经是 Codex 的扫描目录"), "{e}");
        assert!(err(&mut cfg, "claude", s(&codex_skills)).contains("已经是 Codex 的扫描目录"));
        assert!(err(&mut cfg, "codex", s(&claude_skills)).contains("已经是 Claude Code 的扫描目录"));
        // 只读子目录
        assert!(err(&mut cfg, "codex", s(&claude_skills.join("synced"))).contains("已经是 Claude Code 的扫描目录"));
        // 项目的 Skills 目录（同一工具与其他工具）
        assert!(err(&mut cfg, "claude", s(&project_claude)).contains("已经是 Claude Code 的扫描目录"));
        assert!(err(&mut cfg, "codex", s(&project_claude)).contains("已经是 Claude Code 的扫描目录"));
        // 大小写与分隔符不同也拦住
        if cfg!(windows) {
            assert!(err(&mut cfg, "claude", s(&agents).replace('\\', "/").to_uppercase()).contains("已经是 Codex 的扫描目录"));
        }
        // 另一个工具已经更换到的目录
        set_global_dir_with(&h, &mut cfg, "claude", Some(&s(&custom))).unwrap();
        assert!(err(&mut cfg, "codex", s(&custom)).contains("已经是 Claude Code 的扫描目录"));
        assert_eq!(cfg.global_override.keys().collect::<Vec<_>>(), vec!["claude"]);
        // 一次保存里新添加的项目也算：同时添加项目并把它的 Skills 目录选作全局目录，整体拒绝
        let other = base.join("other");
        fs::create_dir_all(other.join(".agents").join("skills")).unwrap();
        let input = DirSettings { custom_dirs: None, projects: vec![s(&project), s(&other)], ignore: vec![], globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&other.join(".agents").join("skills"))) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, input).unwrap_err().contains("已经是 Codex 的扫描目录"));
        assert_eq!(cfg.projects, vec![s(&project)]);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn opencode_shared_skill_roots_are_readonly() {
        let base=temp("opencode-shared");let h=homes_at(&base);let cfg=Config::default();let project=base.join("project");
        let list=roots(&h,&cfg,"opencode",&[project.clone()]);
        for suffix in [".agents/skills",".claude/skills"] {
            assert!(list.iter().any(|r|r.path==base.join(suffix)&&r.readonly));
            assert!(list.iter().any(|r|r.path==project.join(suffix)&&r.readonly));
        }
        assert_eq!(write_root(&h,&cfg,"opencode",Scope::Global,None).unwrap().path,base.join(".config/opencode/skills"));
        fs::remove_dir_all(base).ok();
    }

    #[test]
    fn broken_saved_override_is_readonly_and_reported() {
        let base = temp("override-broken");
        let h = homes_at(&base);
        let data = base.join("data");
        let gone = base.join("gone");
        let mut cfg = Config::default();
        cfg.global_override.insert("claude".into(), gone.display().to_string());
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let reason = format!("已更换的目录不存在：{}", gone.display());
        assert!(o.errors.iter().any(|e| e.reason == reason), "{:?}", o.errors);
        let g = o.roots.iter().find(|r| r.scope == Scope::Global).unwrap();
        assert!(g.readonly && g.overridden && !g.exists);
        assert_eq!(o.platforms.iter().find(|p| p.id == "claude").unwrap().global_dir, None);
        // 写入报错，不静默重建
        let input = SaveInput { platform: "claude".into(), scope: Scope::Global, project: None, name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert_eq!(save_with(&h, &mut cfg, input, &data).unwrap_err(), reason);
        let source = base.join("downloads");
        skill(&source, "api-review", &body("api-review"));
        assert_eq!(import_scan_with(&h, &cfg, &source.display().to_string(), "claude", Scope::Global, None).unwrap_err(), reason);
        assert!(!gone.exists(), "不重建已更换的目录");
        // 相对路径同样视为失效
        cfg.global_override.insert("claude".into(), "relative/dir".into());
        assert!(overview_with(&h, &cfg, "claude").unwrap().errors.iter().any(|e| e.reason.contains("已更换的目录不存在：relative/dir")));
        // 恢复默认后正常
        set_global_dir_with(&h, &mut cfg, "claude", None).unwrap();
        assert!(overview_with(&h, &cfg, "claude").unwrap().errors.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn broken_config_is_backed_up_and_never_overwritten() {
        let data = temp("broken-config");
        let file = data.join("skills.json");
        let broken = "{ \"disabled\": [ 损坏";
        fs::write(&file, broken).unwrap();
        let backups = || {
            let mut list: Vec<PathBuf> = fs::read_dir(&data).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("skills.json.broken-")).map(|e| e.path()).collect();
            list.sort();
            list
        };
        let cfg = load_config(&data);
        assert!(cfg.disabled.is_empty(), "读取扫描照常可用（按空配置）");
        assert_eq!(backups().len(), 1);
        assert_eq!(fs::read_to_string(&backups()[0]).unwrap(), broken, "备份内容与原文件一致");
        let err = save_config(&data, &cfg).unwrap_err();
        assert_eq!(err, format!("Skills 配置文件损坏，已备份为 {}，请检查后重试", backups()[0].display()));
        assert_eq!(fs::read_to_string(&file).unwrap(), broken, "不覆盖损坏的文件");
        // 再读一次不重复备份
        let mut cfg = load_config(&data);
        assert_eq!(backups().len(), 1);
        // 停用（要写停用登记）被拒绝并回滚，目录留在原位
        let h = homes_at(&data);
        let dir = skill(&claude_home(&h).join("skills"), "keep-me", &body("keep-me"));
        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &located, "keep-me", false, &data).unwrap_err().contains("Skills 配置文件损坏"));
        assert!(dir.join(SKILL_FILE).is_file() && cfg.disabled.is_empty());
        assert!(config_guard(&data).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), broken);
        // 修好之后恢复正常
        fs::write(&file, "{}").unwrap();
        let cfg = load_config(&data);
        assert!(config_guard(&data).is_ok());
        save_config(&data, &cfg).unwrap();
        // 空文件按空配置处理，不算损坏
        fs::write(&file, "  ").unwrap();
        load_config(&data);
        assert!(config_guard(&data).is_ok());
        assert_eq!(backups().len(), 1);
        fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn save_dir_settings_is_all_or_nothing() {
        let base = temp("dir-settings");
        let h = homes_at(&base);
        let project = base.join("proj");
        let session = base.join("from-session");
        let custom = base.join("my-skills");
        for d in [&project, &session, &custom] {
            fs::create_dir_all(d).unwrap();
        }
        let s = |p: &Path| p.display().to_string();
        let mut cfg = Config { session_projects: vec![s(&session)], ..Default::default() };
        let snapshot = |c: &Config| serde_json::to_string(c).unwrap();
        let before = snapshot(&cfg);
        let ignore = || vec![IgnoreChange { path: s(&session), ignored: true }];

        // 全局目录不存在：项目与忽略状态也不写入
        let bad = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&base.join("missing"))) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, bad).unwrap_err().contains("目录不存在"));
        assert_eq!(snapshot(&cfg), before, "失败时配置保持原样");
        // 项目目录无效：全局目录也不写入
        let bad = DirSettings { custom_dirs: None, projects: vec![s(&base.join("gone"))], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&custom)) }] };
        assert!(save_dir_settings_with(&h, &mut cfg, bad).unwrap_err().contains("项目目录不存在"));
        assert_eq!(snapshot(&cfg), before);

        let good = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: ignore(), globals: vec![GlobalChange { platform: "claude".into(), path: Some(s(&custom)) }] };
        save_dir_settings_with(&h, &mut cfg, good).unwrap();
        assert_eq!(cfg.projects, vec![s(&project)]);
        assert_eq!(cfg.ignored_projects, vec![s(&session)]);
        assert_eq!(cfg.global_override.get("claude"), Some(&s(&custom)));
        // 同一次保存里恢复默认，其余设置不变
        let reset = DirSettings { custom_dirs: None, projects: vec![s(&project)], ignore: vec![], globals: vec![GlobalChange { platform: "claude".into(), path: None }] };
        save_dir_settings_with(&h, &mut cfg, reset).unwrap();
        assert!(cfg.global_override.is_empty());
        assert_eq!((cfg.projects.len(), cfg.ignored_projects.len()), (1, 1));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn roots_count_parked_skills_left_in_old_dir() {
        let base = temp("parked-here");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let dir = skill(&claude_home(&h).join("skills"), "old-one", &body("old-one"));
        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &located, "old-one", false, &data).unwrap();
        let global = |cfg: &Config| overview_with(&h, cfg, "claude").unwrap().roots.into_iter().find(|r| r.scope == Scope::Global).unwrap();
        assert_eq!(global(&cfg).parked_here, 1, "停用区里原位置在默认目录的 1 个");
        let custom = base.join("my-skills");
        fs::create_dir_all(&custom).unwrap();
        set_global_dir_with(&h, &mut cfg, "claude", Some(&custom.display().to_string())).unwrap();
        assert_eq!(global(&cfg).parked_here, 0, "新目录下没有停用的");
        // 启用逻辑不变：已停用的仍显示，原位置还是旧目录
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let parked = o.items.iter().find(|i| i.name == "old-one").unwrap();
        assert_eq!((parked.disabled_by, parked.original_path.as_deref()), (Some("parked"), Some(dir.display().to_string().as_str())));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn parses_frontmatter_and_validates() {
        let f = parse_front("---\nname: api-review\ndescription: \"核对 API\"\nallowed-tools: Read\n---\n# x").unwrap();
        assert_eq!(f.name.as_deref(), Some("api-review"));
        assert_eq!(f.description.as_deref(), Some("核对 API"));
        let folded = parse_front("---\ndescription: >\n  第一行\n  第二行\n---\n").unwrap();
        assert_eq!(folded.description.as_deref(), Some("第一行 第二行"));
        assert!(parse_front("# 没有 frontmatter").is_err());
        assert!(parse_front("---\nname: a\n").is_err());
        let no_name = Ok(Front { name: None, description: Some("d".into()) });
        assert_eq!(validate_front("claude", &no_name), None);
        assert_eq!(validate_front("codex", &no_name).as_deref(), Some("SKILL.md 缺少 name"));
        assert!(valid_name("api-review") && !valid_name("Api") && !valid_name("-a") && !valid_name("a/b") && !valid_name(""));
    }

    #[test]
    fn scans_conflicts_invalid_and_tool_overrides() {
        let base = temp("scan");
        let h = homes_at(&base);
        let project = base.join("proj");
        let global = claude_home(&h).join("skills");
        skill(&global, "frontend-design", &body("frontend-design"));
        skill(&global, "api-contract", "---\nname: api-contract\n---\n");
        skill(&global, "legacy", &body("legacy"));
        skill(&global.join("synced"), "synced-one", &body("synced-one"));
        skill(&project.join(".claude").join("skills"), "frontend-design", &body("frontend-design"));
        let settings = r#"{"skillOverrides":{"legacy":"off"}}"#;
        fs::write(claude_home(&h).join("settings.json"), settings).unwrap();
        let cfg = Config { projects: vec![project.display().to_string()], ..Default::default() };
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let get = |name: &str, scope: Scope| o.items.iter().find(|i| i.name == name && i.scope == scope).unwrap();
        assert_eq!(get("frontend-design", Scope::Global).status, "enabled");
        let project_item = get("frontend-design", Scope::Project);
        assert_eq!((project_item.status, project_item.issue.as_deref()), ("conflict", Some("同名条目：项目与全局同时存在")));
        assert_eq!(get("api-contract", Scope::Global).status, "invalid");
        let legacy = get("legacy", Scope::Global);
        assert_eq!((legacy.status, legacy.disabled_by, legacy.enabled), ("disabled", Some("tool"), false));
        assert!(legacy.issue.as_deref().unwrap().contains("由工具配置停用"));
        assert!(get("synced-one", Scope::Synced).readonly);
        // 用户写的 skillOverrides 不允许 Jolu 改：启用报错，文件原样
        let mut cfg = cfg;
        let (root, dir) = locate(&h, &cfg, &legacy.path).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &dir, "legacy", true, &base.join("data")).unwrap_err().contains("由工具配置停用"));
        assert_eq!(fs::read_to_string(claude_home(&h).join("settings.json")).unwrap(), settings);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn claude_disable_parks_only_this_copy() {
        let base = temp("park");
        let h = homes_at(&base);
        let data = base.join("data");
        let project = base.join("proj");
        skill(&claude_home(&h).join("skills"), "docs-writer", &body("docs-writer"));
        let local = skill(&project.join(".claude").join("skills"), "docs-writer", &body("docs-writer"));
        fs::write(local.join("notes.md"), "附带文件").unwrap();
        let mut cfg = Config { projects: vec![project.display().to_string()], ..Default::default() };

        let (root, dir) = locate(&h, &cfg, &local.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", false, &data).unwrap();
        assert!(!local.exists() && Path::new(&stored).join("notes.md").is_file() && Path::new(&stored).join(SKILL_FILE).is_file());
        assert!(stored.starts_with(&data.join("skills-disabled").display().to_string()));
        assert!(!claude_home(&h).join("settings.json").exists(), "不写 skillOverrides");
        assert_eq!(load_config(&data).disabled.len(), 1);
        assert!(fs::read_dir(data.join("skill-backups").join("disabled")).unwrap().count() == 1);

        let o = overview_with(&h, &cfg, "claude").unwrap();
        let global_item = o.items.iter().find(|i| i.scope == Scope::Global).unwrap();
        assert_eq!(global_item.status, "enabled", "同名的全局副本不受影响");
        let parked = o.items.iter().find(|i| i.disabled_by == Some("parked")).unwrap();
        assert_eq!((parked.status, parked.original_path.as_deref()), ("disabled", Some(local.display().to_string().as_str())));
        assert!(detail_with(&h, &cfg, &parked.path).is_ok());

        // 原位置被占用时拒绝启用，两边文件都保留
        fs::create_dir_all(&local).unwrap();
        let (root, dir) = locate(&h, &cfg, &stored).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", true, &data).unwrap_err().contains("原位置已有同名目录"));
        assert!(Path::new(&stored).join(SKILL_FILE).is_file());
        fs::remove_dir(&local).unwrap();
        let back = set_enabled_impl(&h, &mut cfg, &root, &dir, "docs-writer", true, &data).unwrap();
        assert!(same_path(&back, &local) && local.join(SKILL_FILE).is_file() && !Path::new(&stored).exists());
        assert!(load_config(&data).disabled.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    fn drive_of(path: &Path) -> Option<String> {
        match path.components().next() {
            Some(std::path::Component::Prefix(p)) => Some(p.as_os_str().to_string_lossy().to_uppercase()),
            _ => None,
        }
    }

    /// Skills 目录在仓库所在盘（D:），停用区在系统临时目录（C:）：停用、启用都要走「复制 + 校验 + 删除」路径，文件内容不变
    #[test]
    fn claude_park_and_unpark_across_volumes() {
        let data = temp("cross-data");
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("jolu-skills-cross-{}-{}", std::process::id(), stamp()));
        if drive_of(&data) == drive_of(&base) {
            eprintln!("未测：临时目录与仓库在同一个盘（{:?}），无法模拟跨盘移动", drive_of(&data));
            fs::remove_dir_all(&data).ok();
            return;
        }
        let h = homes_at(&base);
        let dir = skill(&claude_home(&h).join("skills"), "cross-disk", &body("cross-disk"));
        fs::create_dir_all(dir.join("refs")).unwrap();
        fs::write(dir.join("refs").join("guide.md"), "跨盘附带文件\n第二行").unwrap();
        let bytes = vec![0u8, 1, 2, 254, 255];
        fs::write(dir.join("blob.bin"), &bytes).unwrap();
        let mut cfg = Config::default();

        let (root, located) = locate(&h, &cfg, &dir.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &located, "cross-disk", false, &data).unwrap();
        let stored = PathBuf::from(stored);
        assert_ne!(drive_of(&stored), drive_of(&dir), "停用区应在另一个盘");
        assert!(!dir.exists(), "原目录已删除");
        assert_eq!(fs::read_to_string(stored.join(SKILL_FILE)).unwrap(), body("cross-disk"));
        assert_eq!(fs::read_to_string(stored.join("refs").join("guide.md")).unwrap(), "跨盘附带文件\n第二行");
        assert_eq!(fs::read(stored.join("blob.bin")).unwrap(), bytes);

        let (root, located) = locate(&h, &cfg, &stored.display().to_string()).unwrap();
        let back = set_enabled_impl(&h, &mut cfg, &root, &located, "cross-disk", true, &data).unwrap();
        assert!(same_path(&back, &dir) && !stored.exists());
        assert_eq!(fs::read_to_string(dir.join(SKILL_FILE)).unwrap(), body("cross-disk"));
        assert_eq!(fs::read_to_string(dir.join("refs").join("guide.md")).unwrap(), "跨盘附带文件\n第二行");
        assert_eq!(fs::read(dir.join("blob.bin")).unwrap(), bytes);
        assert!(load_config(&data).disabled.is_empty());
        fs::remove_dir_all(&base).ok();
        fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn codex_toggles_by_path_and_keeps_name_entries() {
        let base = temp("codex");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let a = skill(&base.join(".agents").join("skills"), "test-runner", &body("test-runner"));
        skill(&codex_home(&h).join("skills"), "test-runner", &body("test-runner"));
        skill(&codex_home(&h).join("skills"), "legacy", &body("legacy"));
        fs::create_dir_all(&codex_home(&h)).unwrap();
        fs::write(codex_home(&h).join("config.toml"), "# 我的配置\nmodel = \"gpt-5\"\n\n[[skills.config]]\nname = \"legacy\"\nenabled = false\n").unwrap();
        let (root, dir) = locate(&h, &cfg, &a.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &dir, "test-runner", false, &data).unwrap();
        let text = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
        assert!(text.contains("# 我的配置") && text.contains("name = \"legacy\"") && text.contains("enabled = false"));
        let o = overview_with(&h, &cfg, "codex").unwrap();
        let states: Vec<_> = o.items.iter().map(|i| (i.name.as_str(), i.path == a.display().to_string(), i.disabled_by)).collect();
        assert!(states.contains(&("test-runner", true, Some("path"))));
        assert!(states.contains(&("test-runner", false, None)), "另一份同名副本保持启用");
        assert!(states.contains(&("legacy", false, Some("tool"))));
        let legacy = o.items.iter().find(|i| i.name == "legacy").unwrap();
        let (lroot, ldir) = locate(&h, &cfg, &legacy.path).unwrap();
        assert!(set_enabled_impl(&h, &mut cfg, &lroot, &ldir, "legacy", true, &data).is_err());
        set_enabled_impl(&h, &mut cfg, &root, &dir, "test-runner", true, &data).unwrap();
        let text = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
        assert!(text.contains("name = \"legacy\"") && !text.contains("test-runner"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn merges_session_projects_and_ignores() {
        let base = temp("projects");
        let h = homes_at(&base);
        let with_skills = base.join("WithSkills");
        skill(&with_skills.join(".agents").join("skills"), "repo-skill", &body("repo-skill"));
        let plain = base.join("plain");
        fs::create_dir_all(&plain).unwrap();
        let alt = with_skills.display().to_string().replace('\\', "/").to_uppercase();
        let mut cfg = Config {
            projects: vec![with_skills.display().to_string()],
            session_projects: vec![alt, plain.display().to_string(), base.join("gone").display().to_string(), base.display().to_string()],
            ..Default::default()
        };
        let all = cfg.all_projects(&h);
        assert_eq!(all.len(), 2, "大小写与分隔符去重、跳过不存在目录和主目录");
        assert_eq!((all[0].source, all[0].tools.as_slice()), ("manual", ["codex", "opencode"].as_slice()));
        assert_eq!((all[1].source, all[1].tools.is_empty()), ("session", true));
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert!(o.roots.iter().all(|r| r.scope != Scope::Project || r.exists), "没有 Skills 目录的项目不扫描");
        assert_eq!(o.items.iter().filter(|i| i.scope == Scope::Project).count(), 1);
        // 没有 Skills 目录的会话项目仍可作为新建目标
        let input = SaveInput { platform: "claude".into(), scope: Scope::Project, project: Some(plain.display().to_string()), name: "new-one".into(), content: body("new-one"), original_path: None, expected_modified_ms: None, enable: true };
        assert!(save_with(&h, &mut cfg, input, &base.join("data")).is_ok());
        ignore_project_with(&mut cfg, &plain.display().to_string().to_uppercase(), true);
        let plain_info = cfg.all_projects(&h).into_iter().find(|p| p.source == "session").unwrap();
        assert!(plain_info.ignored);
        assert!(check_project(&h, &cfg, Some(&plain.display().to_string())).is_err());
        ignore_project_with(&mut cfg, &plain.display().to_string(), false);
        assert!(cfg.ignored_projects.is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn save_detects_conflicts_and_external_changes() {
        let base = temp("save");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let content = "---\nname: api-review\ndescription: 核对\n---\n# API Review\n";
        let input = |original: Option<String>, expected: Option<i64>, body: &str, enable: bool| SaveInput {
            platform: "claude".into(),
            scope: Scope::Global,
            project: None,
            name: "api-review".into(),
            content: body.into(),
            original_path: original,
            expected_modified_ms: expected,
            enable,
        };
        let saved = save_with(&h, &mut cfg, input(None, None, content, true), &data).unwrap();
        assert!(save_with(&h, &mut cfg, input(None, None, content, true), &data).unwrap_err().contains("同名"));
        let edited = save_with(&h, &mut cfg, input(Some(saved.path.clone()), saved.modified_ms, &content.replace("核对", "检查"), true), &data).unwrap();
        assert!(edited.backup.is_some());
        assert_eq!(save_with(&h, &mut cfg, input(Some(saved.path.clone()), Some(1), content, true), &data).unwrap_err(), EXTERNAL_CHANGE);
        assert!(save_with(&h, &mut cfg, input(None, None, "---\nname: other\ndescription: d\n---\n", true), &data).unwrap_err().contains("不一致"));
        let mut bad = input(None, None, content, true);
        bad.scope = Scope::Project;
        bad.project = Some(base.join("unknown").display().to_string());
        assert!(save_with(&h, &mut cfg, bad, &data).unwrap_err().contains("目录设置"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn rejects_paths_outside_roots() {
        let base = temp("paths");
        let h = homes_at(&base);
        let cfg = Config::default();
        let dir = skill(&claude_home(&h).join("skills"), "safe", "---\ndescription: d\n---\n");
        let outside = skill(&base.join("elsewhere"), "evil", "---\ndescription: d\n---\n");
        assert!(locate(&h, &cfg, &dir.display().to_string()).is_ok());
        assert!(locate(&h, &cfg, &outside.display().to_string()).is_err());
        assert!(locate(&h, &cfg, &format!("{}/../elsewhere/evil", dir.display())).is_err());
        assert!(locate(&h, &cfg, "relative/path").is_err());
        assert!(file_with(&h, &cfg, &dir.display().to_string(), "../../elsewhere/evil/SKILL.md").is_err());
        assert!(file_with(&h, &cfg, &dir.display().to_string(), "SKILL.md").unwrap().text.is_some());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn imports_disabled_into_parking_and_removes_with_backup() {
        let base = temp("import");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &body("api-review"));
        skill(&source, "frontend-design", &body("frontend-design"));
        skill(&source, "quick-test", "---\nname: quick-test\n---\n");
        skill(&claude_home(&h).join("skills"), "frontend-design", &body("frontend-design"));
        let src = source.display().to_string();
        let scan = import_scan_with(&h, &cfg, &src, "claude", Scope::Global, None).unwrap();
        let status: Vec<_> = scan.items.iter().map(|i| (i.name.as_str(), i.status)).collect();
        assert_eq!(status, vec![("api-review", "ok"), ("frontend-design", "conflict"), ("quick-test", "invalid")]);
        let choices = vec![
            ImportChoice { name: "api-review".into(), action: "import".into() },
            ImportChoice { name: "frontend-design".into(), action: "skip".into() },
            ImportChoice { name: "quick-test".into(), action: "import".into() },
        ];
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, choices, false, &data).unwrap();
        assert_eq!((result.imported.len(), result.skipped.len(), result.failed.len()), (1, 1, 1));
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let imported = o.items.iter().find(|i| i.name == "api-review").unwrap();
        assert_eq!((imported.source, imported.status, imported.disabled_by), ("imported", "disabled", Some("parked")));
        let backup = remove_with(&h, &mut cfg, &imported.path, &data).unwrap();
        assert!(Path::new(&backup).join(SKILL_FILE).is_file());
        assert!(!Path::new(&imported.path).exists() && cfg.disabled.is_empty());
        let copy = copy_with(&h, &cfg, &o.items.iter().find(|i| i.name == "frontend-design").unwrap().path, "codex", Scope::Global, None, false).unwrap();
        assert!(copy.copied && Path::new(&copy.target).join(SKILL_FILE).is_file());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn git_url_rules() {
        assert!(validate_git_url("https://github.com/o/r.git").is_ok());
        assert!(validate_git_url("https://github.com/anthropics/skills").is_ok());
        for bad in ["git@github.com:o/r", "http://x", "https://u:p@x/r", "file:///c:/x", "https://user@x/r", "https://", "https://x/r with space", "ssh://x/r"] {
            assert!(validate_git_url(bad).is_err(), "应拒绝 {bad}");
        }
        assert!(validate_git_branch("main").is_ok() && validate_git_branch("release/v1.2").is_ok());
        assert!(validate_git_branch("-x").is_err() && validate_git_branch("a b").is_err() && validate_git_branch("").is_err());
    }

    #[test]
    fn cleanup_only_removes_own_temp() {
        let base = temp("git-cleanup");
        let temp_root = base.join("tmp");
        let own = temp_root.join("jolu-git-20261023-1");
        fs::create_dir_all(own.join(".git").join("objects")).unwrap();
        let pack = own.join(".git").join("objects").join("pack.idx");
        fs::write(&pack, "x").unwrap();
        // git 把对象文件设为只读，清理时也要删得掉
        let mut perms = fs::metadata(&pack).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&pack, perms).unwrap();
        let other = temp_root.join("other-dir");
        fs::create_dir_all(&other).unwrap();
        let outside = base.join("jolu-git-outside");
        fs::create_dir_all(&outside).unwrap();
        for path in [&other, &outside, &temp_root] {
            assert!(cleanup_checkout(&temp_root, &path.display().to_string()).is_err(), "不应删除 {}", path.display());
            assert!(path.is_dir(), "目录应保持不变：{}", path.display());
        }
        assert!(cleanup_checkout(&temp_root, &format!("{}/../jolu-git-outside", other.display())).is_err());
        assert!(cleanup_checkout(&temp_root, "jolu-git-relative").is_err());
        assert!(outside.is_dir());
        // jolu-git-* 是指向外部目录的 junction 时拒绝删除，外部目录保持不变
        #[cfg(windows)]
        {
            let junction = temp_root.join("jolu-git-junction");
            fs::write(outside.join("keep.txt"), "外部文件").unwrap();
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&outside)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if made {
                assert!(cleanup_checkout(&temp_root, &junction.display().to_string()).is_err(), "junction 不应被清理");
                assert!(junction.exists() && outside.join("keep.txt").is_file());
                fs::remove_dir(&junction).unwrap();
            } else {
                println!("未测：无法创建 junction");
            }
        }
        cleanup_checkout(&temp_root, &own.display().to_string()).unwrap();
        assert!(!own.exists());
        // 已经删掉的再清理一次不报错
        assert!(cleanup_checkout(&temp_root, &own.display().to_string()).is_ok());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn git_missing_reports_install_hint() {
        let base = temp("git-missing");
        let err = clone_repo("jolu-no-such-git-binary", "https://github.com/o/r.git", None, &base.join("jolu-git-x"), Duration::from_secs(5)).unwrap_err();
        assert_eq!(err, GIT_MISSING);
        fs::remove_dir_all(&base).ok();
    }

    /// 联网：`cargo test skills::tests::git_clone_public_repo -- --ignored`
    #[test]
    #[ignore]
    fn git_clone_public_repo() {
        let base = temp("git-net");
        let h = homes_at(&base);
        let temp_root = base.join("tmp");
        let scan = import_git_with(&h, &Config::default(), &temp_root, "git", "https://github.com/anthropics/skills", None, "claude", Scope::Global, None).unwrap();
        assert!(Path::new(&scan.checkout_dir).starts_with(&temp_root));
        assert!(scan.scan.source.replace('\\', "/").ends_with("/skills"), "来源应为仓库里的 skills/：{}", scan.scan.source);
        assert!(scan.scan.items.iter().filter(|i| i.status == "ok").count() > 3);
        let bad = import_git_with(&h, &Config::default(), &temp_root, "git", "https://github.com/anthropics/skills", Some("jolu-no-such-branch"), "claude", Scope::Global, None).unwrap_err();
        assert!(bad.contains("没有这个分支"), "{bad}");
        cleanup_checkout(&temp_root, &scan.checkout_dir).unwrap();
        assert_eq!(fs::read_dir(&temp_root).unwrap().count(), 0, "失败的克隆也应删除临时目录");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn repo_dir_name_from_url() {
        assert_eq!(repo_dir_name("https://github.com/o/my-skill.git"), "my-skill");
        assert_eq!(repo_dir_name("https://github.com/o/my-skill/"), "my-skill");
        assert_eq!(repo_dir_name("https://x/o/r.git?ref=1#top"), "r");
        assert_eq!(repo_dir_name("https://x/o/My%20Repo"), "My_20Repo");
        assert_eq!(repo_dir_name("https://x/o/.."), "_");
    }

    #[test]
    fn git_single_skill_repo_uses_repo_name_and_drops_git_dir() {
        let base = temp("git-single");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let checkout = base.join("tmp").join("jolu-git-20261023-1");
        let repo = checkout.join(repo_dir_name("https://github.com/o/my-skill.git"));
        fs::create_dir_all(repo.join(".git").join("objects")).unwrap();
        fs::write(repo.join(SKILL_FILE), body("my-skill")).unwrap();
        let pack = repo.join(".git").join("objects").join("pack.idx");
        fs::write(&pack, "x").unwrap();
        let mut perms = fs::metadata(&pack).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&pack, perms).unwrap();

        let scan = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap();
        assert!(!repo.join(".git").exists(), "克隆后先删掉 .git");
        let names: Vec<_> = scan.items.iter().map(|i| (i.name.as_str(), i.status)).collect();
        assert_eq!(names, vec![("my-skill", "ok")], "名称是仓库名，不是 jolu-git-*");
        let choices = vec![ImportChoice { name: "my-skill".into(), action: "import".into() }];
        let result = import_with(&h, &mut cfg, &scan.source, "claude", Scope::Global, None, choices, true, &data).unwrap();
        assert_eq!(result.imported.len(), 1);
        let target = claude_home(&h).join("skills").join("my-skill");
        assert!(target.join(SKILL_FILE).is_file() && !target.join(".git").exists(), "目标目录里没有 .git");
        // 再导入一次命中冲突检查
        let again = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap();
        assert_eq!(again.items[0].status, "conflict");

        // 仓库名不符合 Skill 命名规则时标为异常
        let odd = base.join("tmp").join("jolu-git-20261023-2").join(repo_dir_name("https://github.com/o/My.Skill.git"));
        skill(odd.parent().unwrap(), "My.Skill", &body("my-skill"));
        let scan = scan_clone(&h, &cfg, &odd, "claude", Scope::Global, None).unwrap();
        assert_eq!(scan.items[0].status, "invalid");
        assert!(scan.items[0].issue.as_deref().unwrap().contains("目录名不符合 Skill 名称规则"));
        fs::remove_dir_all(&base).ok();
    }

    fn new_body(name: &str) -> String {
        format!("---\nname: {name}\ndescription: 新版内容\n---\n")
    }

    fn overwrite(name: &str) -> Vec<ImportChoice> {
        vec![ImportChoice { name: name.into(), action: "overwrite".into() }]
    }

    #[test]
    fn overwrite_rejects_its_own_source_without_removing_files() {
        let base = temp("overwrite-self");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dir = skill(&root.path, "demo", &original);
        fs::write(dir.join("notes.txt"), "保留附件").unwrap();
        let result = import_with(&h, &mut cfg, &dir.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1, "不能把安装项覆盖到自身");
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dir.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "保留附件");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_rejects_nested_source_without_removing_files() {
        let base = temp("overwrite-nested");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dest = skill(&root.path, "demo", &original);
        let replacement = format!("{original}\n新正文");
        let source = skill(&dest.join("nested"), "demo", &replacement);
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(source.join(SKILL_FILE)).unwrap(), replacement);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_replaces_tree_and_keeps_backup_and_source() {
        let base = temp("overwrite-tree");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = body("demo");
        let dest = skill(&root.path, "demo", &original);
        fs::write(dest.join("old.txt"), "旧附件").unwrap();
        let replacement = format!("{original}\n新正文");
        let source = skill(&base.join("source"), "demo", &replacement);
        fs::write(source.join("new.txt"), "新附件").unwrap();
        let data = base.join("data");
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &data).unwrap();
        assert!(result.failed.is_empty());
        assert_eq!(result.imported.len(), 1);
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), replacement);
        assert_eq!(fs::read_to_string(source.join(SKILL_FILE)).unwrap(), replacement);
        assert_eq!(fs::read_to_string(dest.join("new.txt")).unwrap(), "新附件");
        assert!(!dest.join("old.txt").exists());
        let backup = fs::read_dir(data.join("skill-backups/overwritten")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::read_to_string(backup.join(SKILL_FILE)).unwrap(), original);
        assert_eq!(fs::read_to_string(backup.join("old.txt")).unwrap(), "旧附件");
        assert_eq!(fs::read_dir(&root.path).unwrap().count(), 1, "成功后不残留暂存目录");
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn overwrite_copy_failure_preserves_original_installation() {
        use std::os::windows::fs::OpenOptionsExt;
        let base = temp("overwrite-failed-copy");
        let h = homes_at(&base);
        let mut cfg = Config::default();
        let root = write_root(&h, &cfg, "claude", Scope::Global, None).unwrap();
        let original = format!("{}\n旧正文", body("demo"));
        let dest = skill(&root.path, "demo", &original);
        fs::write(dest.join("notes.txt"), "旧附件").unwrap();
        let source = skill(&base.join("source"), "demo", &format!("{}\n新正文", body("demo")));
        fs::write(source.join("locked.txt"), "读取失败").unwrap();
        let locked = fs::OpenOptions::new().read(true).share_mode(0).open(source.join("locked.txt")).unwrap();
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, overwrite("demo"), true, &base.join("data")).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(result.imported.is_empty());
        assert_eq!(fs::read_to_string(dest.join(SKILL_FILE)).unwrap(), original, "复制失败不改变现有正文");
        assert_eq!(fs::read_to_string(dest.join("notes.txt")).unwrap(), "旧附件");
        drop(locked);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overwrite_keeps_enabled_state() {
        let base = temp("ow-enabled");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &new_body("api-review"));
        let src = source.display().to_string();

        // Claude：启用中的 Skill 被覆盖后仍在原位置启用，「导入后启用」关闭也不停用
        let target = skill(&claude_home(&h).join("skills"), "api-review", &body("api-review"));
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, overwrite("api-review"), false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert!(same_path(&result.imported[0], &target));
        assert_eq!(fs::read_to_string(target.join(SKILL_FILE)).unwrap(), new_body("api-review"), "内容已更新");
        assert!(cfg.disabled.is_empty() && !data.join("skills-disabled").exists(), "没有移进停用区");
        let o = overview_with(&h, &cfg, "claude").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().status, "enabled");

        // Codex：启用中的 Skill 被覆盖后 config.toml 不出现停用条目
        let codex_target = skill(&base.join(".agents").join("skills"), "api-review", &body("api-review"));
        let result = import_with(&h, &mut cfg, &src, "codex", Scope::Global, None, overwrite("api-review"), false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(fs::read_to_string(codex_target.join(SKILL_FILE)).unwrap(), new_body("api-review"));
        assert!(!codex_home(&h).join("config.toml").exists(), "不写停用条目");
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().disabled_by, None);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn overwrite_keeps_disabled_state() {
        let base = temp("ow-disabled");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "api-review", &new_body("api-review"));
        let src = source.display().to_string();

        // Claude：停用区里的 Skill 算作同名冲突，覆盖写到停用区里那一份，不在原位置再放一份
        let original = skill(&claude_home(&h).join("skills"), "api-review", &body("api-review"));
        let (root, dir) = locate(&h, &cfg, &original.display().to_string()).unwrap();
        let stored = set_enabled_impl(&h, &mut cfg, &root, &dir, "api-review", false, &data).unwrap();
        let registry = serde_json::to_string(&load_config(&data).disabled).unwrap();
        let scan = import_scan_with(&h, &cfg, &src, "claude", Scope::Global, None).unwrap();
        assert_eq!(scan.items[0].status, "conflict", "停用区里的同名 Skill 也是冲突");
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, overwrite("api-review"), true, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert!(same_path(&result.imported[0], Path::new(&stored)));
        assert!(!original.exists(), "原位置没有第二份");
        assert_eq!(fs::read_to_string(Path::new(&stored).join(SKILL_FILE)).unwrap(), new_body("api-review"), "停用区里的内容已更新");
        assert_eq!(serde_json::to_string(&cfg.disabled).unwrap(), registry, "停用登记不变");
        assert!(fs::read_dir(data.join("skill-backups").join("overwritten")).unwrap().count() == 1, "覆盖前已备份");
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let items: Vec<_> = o.items.iter().filter(|i| i.name == "api-review").collect();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].status, items[0].disabled_by), ("disabled", Some("parked")));
        // 停用区里的同名 Skill 选「导入」也要报冲突，不能在原位置再放一份
        let choices = vec![ImportChoice { name: "api-review".into(), action: "import".into() }];
        let result = import_with(&h, &mut cfg, &src, "claude", Scope::Global, None, choices, true, &data).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(!original.exists());

        // Codex：按路径停用的 Skill 被覆盖后仍停用，config.toml 原样
        let codex_target = skill(&base.join(".agents").join("skills"), "api-review", &body("api-review"));
        let (root, dir) = locate(&h, &cfg, &codex_target.display().to_string()).unwrap();
        set_enabled_impl(&h, &mut cfg, &root, &dir, "api-review", false, &data).unwrap();
        let config = fs::read_to_string(codex_home(&h).join("config.toml")).unwrap();
        let result = import_with(&h, &mut cfg, &src, "codex", Scope::Global, None, overwrite("api-review"), true, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(fs::read_to_string(codex_target.join(SKILL_FILE)).unwrap(), new_body("api-review"));
        assert_eq!(fs::read_to_string(codex_home(&h).join("config.toml")).unwrap(), config, "config.toml 不变");
        let o = overview_with(&h, &cfg, "codex").unwrap();
        assert_eq!(o.items.iter().find(|i| i.name == "api-review").unwrap().disabled_by, Some("path"));
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn enable_after_import_only_for_new() {
        let base = temp("ow-new");
        let h = homes_at(&base);
        let data = base.join("data");
        let mut cfg = Config::default();
        let source = base.join("downloads");
        skill(&source, "fresh-one", &body("fresh-one"));
        skill(&source, "kept-on", &new_body("kept-on"));
        let existing = skill(&claude_home(&h).join("skills"), "kept-on", &body("kept-on"));
        let choices = vec![
            ImportChoice { name: "fresh-one".into(), action: "import".into() },
            ImportChoice { name: "kept-on".into(), action: "overwrite".into() },
        ];
        let result = import_with(&h, &mut cfg, &source.display().to_string(), "claude", Scope::Global, None, choices, false, &data).unwrap();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        let o = overview_with(&h, &cfg, "claude").unwrap();
        let state = |name: &str| o.items.iter().find(|i| i.name == name).map(|i| (i.status, i.disabled_by)).unwrap();
        assert_eq!(state("fresh-one"), ("disabled", Some("parked")), "新导入的按开关停用");
        assert_eq!(state("kept-on"), ("enabled", None), "覆盖的保持原状态");
        assert_eq!(fs::read_to_string(existing.join(SKILL_FILE)).unwrap(), new_body("kept-on"));
        assert_eq!(cfg.disabled.len(), 1);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn scan_clone_stops_when_git_removal_unconfirmed() {
        let base = temp("git-unknown");
        let h = homes_at(&base);
        let cfg = Config::default();
        // 路径里的 NUL 让 symlink_metadata 返回 InvalidInput（不是 NotFound），无法确认 .git 是否还在
        let repo = base.join("bad\0repo");
        let err = scan_clone(&h, &cfg, &repo, "claude", Scope::Global, None).unwrap_err();
        assert!(err.starts_with("无法确认 .git 已删除："), "{err}");
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn git_import_source_prefers_root_then_skills_dir() {
        let base = temp("git-source");
        let flat = base.join("flat");
        skill(&flat, "a-skill", &body("a-skill"));
        assert_eq!(git_import_source(&flat), flat);
        let nested = base.join("nested");
        skill(&nested.join("skills"), "b-skill", &body("b-skill"));
        skill(&nested, "template", &body("template"));
        assert_eq!(git_import_source(&nested), nested.join("skills"));
        let single = base.join("single");
        fs::create_dir_all(&single).unwrap();
        fs::write(single.join(SKILL_FILE), body("single")).unwrap();
        assert_eq!(git_import_source(&single), single);
        let empty = base.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(git_import_source(&empty), empty);
        fs::remove_dir_all(&base).ok();
    }

// 任务 3.3 开始
#[cfg(test)]
mod task_3_3_tests {
    use super::*;

    fn skill(root: &Path, name: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SKILL_FILE), body).unwrap();
    }

    #[test]
    fn enabled_skills_lists_global_and_cwd_project() {
        let base = std::env::temp_dir().join(format!("jolu-skills-enabled-{}-{}", std::process::id(), stamp()));
        let h = homes_from(base.clone(), |_| None);
        let claude_home = tool_home(&h, spec("claude").unwrap());
        let global = claude_home.join("skills");
        let body = |n: &str| format!("---\nname: {n}\ndescription: {n} 描述\n---\n");
        skill(&global, "api-review", &body("api-review"));
        skill(&global, "no-desc", "---\nname: no-desc\n---\n");
        skill(&global, "Upper", "---\ndescription: 目录名不合规\n---\n");
        skill(&global, "turned-off", &body("turned-off"));
        fs::write(claude_home.join("settings.json"), r#"{"skillOverrides":{"turned-off":"off"}}"#).unwrap();
        let project = base.join("proj");
        skill(&project.join(".claude").join("skills"), "test-check", &body("test-check"));
        skill(&project.join(".claude").join("skills"), "api-review", &body("api-review"));
        let other = base.join("other");
        skill(&other.join(".claude").join("skills"), "elsewhere", &body("elsewhere"));

        let names = |list: Vec<SkillOption>| list.into_iter().map(|o| (o.name, o.scope, o.project.is_some())).collect::<Vec<_>>();
        assert_eq!(
            names(enabled_skills_with(&h, "claude", Some(&project)).unwrap()),
            vec![("api-review".into(), "global".into(), false), ("test-check".into(), "project".into(), true)],
            "只列已启用、有效、名称合规的；同名只留一份；不扫描其他项目",
        );
        assert_eq!(names(enabled_skills_with(&h, "claude", None).unwrap()), vec![("api-review".into(), "global".into(), false)]);
        let first = enabled_skills_with(&h, "claude", Some(&project)).unwrap().remove(0);
        assert_eq!(first.description.as_deref(), Some("api-review 描述"));
        assert!(enabled_skills_with(&h, "codex", Some(&project)).unwrap().is_empty(), "按工具区分");
        assert!(enabled_skills_with(&h, "all", None).is_err());
        assert!(enabled_skills_with(&h, "gemini", None).is_err());
        fs::remove_dir_all(&base).ok();
    }
}
// 任务 3.3 结束
