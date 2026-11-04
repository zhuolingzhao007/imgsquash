import { mount, setColorScheme, View } from "nsh";
import { Window } from "nsh/desktop";
import { colors as c } from "./ui";
import { addFiles, busy, files, filter, importing } from "./state/queue";
import { bindShortcuts } from "./state/shortcuts";
import { dragging, settingsOpen } from "./state/view";
import { layout, measureSurface, MIN_HEIGHT, MIN_WIDTH } from "./state/layout";
import { renderTopBar } from "./components/TopBar";
import { renderPageHeader } from "./components/PageHeader";
import { renderSidebar } from "./components/Sidebar";
import { renderDropzone } from "./components/Dropzone";
import { renderNotice } from "./components/Notice";
import { renderQueue } from "./components/Queue";
import { renderBottomBar } from "./components/BottomBar";
import { renderSettings } from "./components/Settings";

Window.setTitle("ImgSquash · 图片压缩");
Window.setTitleBar("hidden", 48);
Window.setFrame({ size: { width: 1180, height: 800 } });
setColorScheme("light");
bindShortcuts();

// 已经为最小宽度拉过一次窗口就不再重复，免得宿主不接受时来回拉锯。
let floorRequested = false;

/**
 * 三栏骨架。整体放在一个 render 函数里由 App 内联调用：布局档位读的是
 * 尺寸信号，必须落在渲染作用域里，窗口一缩放才能跟着重排。
 */
function renderShell() {
  const l = layout();
  return <>
    {renderTopBar({ settingsOpen: settingsOpen(), onOpenSettings: () => settingsOpen.update(open => !open) })}
    <View style={{ flex: 1, flexDirection: "row", minHeight: 0, position: "relative" }}>
      {l.sidebar === "full" ? renderSidebar() : renderSidebar({ rail: true })}
      <View style={{ flex: 1, minWidth: 0, paddingHorizontal: l.gutter, paddingTop: l.short ? 18 : 26 }}>
        {renderPageHeader()}
        {(filter() === "all" || dragging()) && renderDropzone({ busy: busy(), hasFiles: files().length > 0, importing: importing() })}
        {renderNotice()}
        {renderQueue()}
        {renderBottomBar()}
      </View>
      {l.docked && renderSettings({ busy: busy() })}
      {!l.docked && settingsOpen() && <View style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, zIndex: 20 }}>
        <View onPress={() => settingsOpen.set(false)} focusable={false} style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, backgroundColor: "#25372E22" }} />
        {renderSettings({ busy: busy(), drawer: true })}
      </View>}
    </View>
  </>;
}

export default function App() {
  return <View style={{ flex: 1, backgroundColor: c.canvas }}
    onLayout={b => {
      measureSurface(b.size.width, b.size.height);
      // 窗口没有最小尺寸的接口，量到比下限还小就把窗口拉回去。
      // 宽高一起判断、一次拉完，免得两次 setFrame 互相覆盖。
      const belowWidth = b.size.width < MIN_WIDTH - 1;
      const belowHeight = b.size.height < MIN_HEIGHT - 1;
      if (belowWidth || belowHeight) {
        if (!floorRequested) {
          floorRequested = true;
          Window.setFrame({
            size: {
              width: Math.max(b.size.width, MIN_WIDTH),
              height: Math.max(b.size.height, MIN_HEIGHT),
            },
          });
        }
      } else {
        floorRequested = false;
      }
    }}
    onFileDragEnter={() => { if (!busy()) dragging.set(true); }}
    onFileDragLeave={() => dragging.set(false)}
    onFileDrop={e => { dragging.set(false); void addFiles(e.uris); }}>
    {renderShell()}
  </View>;
}

mount(App);
