// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/X3PolicyExecutor.sol";

contract X3ActionVectorTest {
    function testCompilerActionVector() public {
        X3PolicyExecutor e = new X3PolicyExecutor(address(this));
        X3PolicyExecutor.Action memory a = X3PolicyExecutor.Action(
            84532,
            address(0x1111111111111111111111111111111111111111),
            address(0x2222222222222222222222222222222222222222),
            0x12345678,
            0x30ca65d5da355227c97ff836c9c6719af9d3835fc6bc72bddc50eeecc1bb2b25,
            4,
            address(0x3333333333333333333333333333333333333333),
            0xfd6009170f098f674bbf34dcbad63fb3ef9a55b02624369f4cef2d0ca83d226f,
            2, 9, 2000
        );
        require(e.actionId(a) == 0xf1378e67992f207c9569ea986c2c9e003acc511b061be03ecd7ceffde9d8de8e, "compiler action ID differs");
    }
}
