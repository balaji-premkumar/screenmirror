import 'package:flutter_test/flutter_test.dart';

import 'package:stream_mobile_app/main.dart';

void main() {
  testWidgets('App renders without crashing', (WidgetTester tester) async {
    await tester.pumpWidget(const MirrorCompanionApp());

    // Verify the app title renders
    expect(find.text('MIRROR'), findsOneWidget);
    expect(find.text('COMPANION'), findsOneWidget);
  });
}
