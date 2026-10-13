//! "Send with The Tunnel" in the file manager's right-click menu.
//!
//! macOS: a Quick Action (Automator service) in ~/Library/Services, which shows
//! under Quick Actions / Services when right-clicking in Finder. It needs no code
//! signing, unlike a Finder extension.
//! Windows: per-user registry verbs for files and folders. On Windows 11 they
//! appear under "Show more options" until the app is signed and packaged.
//!
//! Both launch the app with `--send <paths…>`; the single-instance plugin hands
//! those paths to the copy that's already running.

pub const MENU_TITLE: &str = "Send with The Tunnel";

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSBackgroundColorName</key>
			<string>background</string>
			<key>NSIconName</key>
			<string>NSActionTemplate</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>Send with The Tunnel</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.item</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#;

    const DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>open -n -b com.thetunnel.app --args --send "$@"</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/zsh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>6F1D1D53-2B4A-4C47-9B5C-3E7C1A6E1001</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
				</array>
				<key>OutputUUID</key>
				<string>6F1D1D53-2B4A-4C47-9B5C-3E7C1A6E1002</string>
				<key>UUID</key>
				<string>6F1D1D53-2B4A-4C47-9B5C-3E7C1A6E1003</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict/>
				<key>isViewVisible</key>
				<integer>1</integer>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleID</key>
		<string>com.apple.finder</string>
		<key>applicationBundleIDsByPath</key>
		<dict>
			<key>/System/Library/CoreServices/Finder.app</key>
			<string>com.apple.finder</string>
		</dict>
		<key>applicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>applicationPaths</key>
		<array>
			<string>/System/Library/CoreServices/Finder.app</string>
		</array>
		<key>serviceApplicationBundleID</key>
		<string>com.apple.finder</string>
		<key>serviceApplicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>15</integer>
		<key>processesInput</key>
		<false/>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<false/>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<false/>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#;

    fn workflow_dir() -> Option<PathBuf> {
        Some(dirs_home()?.join("Library/Services").join(format!("{}.workflow", super::MENU_TITLE)))
    }

    fn dirs_home() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }

    pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
        let Some(dir) = workflow_dir() else { return Ok(()) };
        if enabled {
            let contents = dir.join("Contents");
            std::fs::create_dir_all(&contents)?;
            let unchanged = std::fs::read_to_string(contents.join("document.wflow")).is_ok_and(|d| d == DOCUMENT);
            std::fs::write(contents.join("Info.plist"), INFO_PLIST)?;
            std::fs::write(contents.join("document.wflow"), DOCUMENT)?;
            if !unchanged {
                refresh_services();
            }
        } else if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            refresh_services();
        }
        Ok(())
    }

    fn refresh_services() {
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
    }
}

#[cfg(windows)]
mod imp {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const KEYS: &[&str] = &[r"Software\Classes\*\shell\TheTunnel", r"Software\Classes\Directory\shell\TheTunnel"];

    pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if !enabled {
            for key in KEYS {
                let _ = hkcu.delete_subkey_all(key);
            }
            return Ok(());
        }
        let exe = std::env::current_exe()?;
        let exe = exe.display();
        for key in KEYS {
            let (verb, _) = hkcu.create_subkey(key)?;
            verb.set_value("MUIVerb", &super::MENU_TITLE)?;
            verb.set_value("Icon", &format!("\"{exe}\",0"))?;
            // One process per selected item; the running app batches them into one send.
            verb.set_value("MultiSelectModel", &"Player")?;
            let (command, _) = verb.create_subkey("command")?;
            command.set_value("", &format!("\"{exe}\" --send \"%1\""))?;
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub fn set_enabled(_enabled: bool) -> std::io::Result<()> {
        Ok(())
    }
}

pub use imp::set_enabled;
