#!/usr/bin/python
"""
Author: Vertigo Designs, Ryan Summers

Description: Provides an API for controlling Booster NGFW over MQTT.
"""
import asyncio
import json
import enum

from gmqtt import Client as MqttClient
import miniconf

# A list of channel enumeration names. The index in the list corresponds with the channel name.
CHANNEL = [
    "Zero",
    "One",
    "Two",
    "Three",
    "Four",
    "Five",
    "Six",
    "Seven",
]

class Action(enum.Enum):
    """ Represents an action that can be taken on channel state. """
    ReadBiasCurrent = 'ReadBiasCurrent'
    Save = 'Save'


class BoosterApi:
    """ An asynchronous API for controlling booster using the MQTT control interface. """

    @classmethod
    async def create(cls, prefix, broker):
        """ Create a connection to MQTT for communication with booster. """
        # If the user did not provide a prefix, try to find one.
        if not prefix:
            devices = await miniconf.discover(broker, 'dt/sinara/booster/+')

            if not devices:
                raise Exception('No Boosters found')

            assert len(devices) == 1, 'Multiple Boosters found: {devices}. Please specify one with --prefix'

            prefix = devices[0]

        settings_interface = await miniconf.Miniconf.create(prefix, broker)
        client = MqttClient(client_id='')
        await client.connect(broker)
        client.subscribe(f"{prefix}/control/response")
        return cls(client, prefix, settings_interface)


    def __init__(self, client, prefix, settings_interface):
        """ Consructor.

        Args:
            client: A connected MQTT5 client.
            prefix: The prefix of the booster to control.
        """
        self.client = client
        self.prefix = prefix
        self.command_complete = asyncio.Event()
        self.client.on_message = self._handle_response
        self.response = None
        self.settings_interface = settings_interface


    def _handle_response(self, client, topic, payload, *_args, **_kwargs):
        """ Callback function for when messages are received over MQTT.

        Args:
            client: The MQTT client.
            topic: The topic that the message was received on.
            payload: The payload of the message.
            qos: The quality-of-service of the message.
            properties: Any properties associated with the message.
        """
        if topic != f'{self.prefix}/control/response':
            raise Exception(f'Unknown topic: {topic}')

        # Indicate a response was received.
        self.response = json.loads(payload)
        self.command_complete.set()


    async def perform_action(self, action: Action, channel: str):
        """ Send a command to a booster control topic.

        Args:
            action: The action to take
            channel: The channel on which to perform the action.

        Returns:
            The received response to the action.
        """
        self.command_complete.clear()
        message = json.dumps({
            'channel': CHANNEL[channel],
            'action': action.value,
        })
        self.client.publish(
            f'{self.prefix}/control', payload=message, qos=0, retain=False,
            response_topic=f'{self.prefix}/control/response')
        await self.command_complete.wait()

        # Check the response code.
        assert self.response['code'] == 200, f'Request failed: {self.response}'
        response = self.response
        self.response = None
        return response


    async def tune_bias(self, channel, current):
        """ Set a booster RF bias current.

        Args:
            channel: The channel index to configure.
            current: The bias current.

        Returns:
            (Vgs, Ids) where Vgs is the actual bias voltage and Ids is
            the measured RF amplifier drain current.
        """
        # Power up the channel. Wait for the channel to fully power-up before continuing.
        await self.settings_interface.command(f'channel/{channel}/state', "Powered", retain=False)
        await asyncio.sleep(0.4)

        async def set_bias(voltage):
            await self.settings_interface.command(f'channel/{channel}/bias_voltage',
                                                  voltage, retain=False)
            # Sleep 100 ms for bias current to settle and for ADC to take current measurement.
            await asyncio.sleep(0.1)
            response = await self.perform_action(Action.ReadBiasCurrent, channel)
            response = json.loads(response['msg'])
            vgs, ids = response['vgs'], response['ids']
            print(f'Vgs = {vgs:.3f} V, Ids = {ids * 1000:.2f} mA')
            return vgs, ids

        # v_gsq from datasheet
        voltage = -2.1
        vgs_max = -0.3
        ids_max = .2

        # scan upwards in steps of 20 mV to just above target
        last_ids = 0.
        while True:
            if voltage > vgs_max:
                raise ValueError(f'Voltage out of bounds')
            vgs, ids = await set_bias(voltage)
            if ids > ids_max:
                raise ValueError(f'Ids out of range')
            if ids < last_ids - .02:
                raise ValueError(f'Foldback')
            last_ids = ids
            if ids > current:
                break
            voltage += .02
        vgs_max = voltage

        # scan downwards in steps of 1 mV to just below target
        while True:
            voltage -= .001
            if not vgs_max - .03 <= voltage <= vgs_max:
                raise ValueError(f'Voltage out of bounds')
            vgs, ids = await set_bias(voltage)
            if ids > ids_max:
                raise ValueError(f'Ids out of range')
            if ids <= current:
                break

        return vgs, ids
