import { Text, View } from "nsh";
import { Button } from "nsh/base";
import { colors as c, Icon } from "../ui";
import { notice } from "../state/queue";

export function renderNotice() {
  if (!notice()) return null;
  return <View role="alert" style={{ flexDirection: "row", alignItems: "center", gap: 8, padding: 10, marginTop: 12, borderRadius: 7, backgroundColor: "#FCF1E9" }}>
    <Icon name="info" size={15} color="#A97852" />
    <Text style={{ flex: 1, fontSize: 10, color: "#A97852" }}>{notice()}</Text>
    <Button onPress={() => notice.set("")} accessibilityLabel="关闭提示"><Icon name="close" size={13} color="#A97852" /></Button>
  </View>;
}
